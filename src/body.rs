use crate::{balance::Lease, metrics::RouteMetrics};
use axum::body::Body;
use bytes::Bytes;
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Instant,
};
use tokio::{
    sync::OwnedSemaphorePermit,
    time::{Instant as TokioInstant, Sleep},
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub struct Observation {
    pub metrics: Arc<RouteMetrics>,
    started: Instant,
    pub permit: Option<OwnedSemaphorePermit>,
    pub lease: Option<Lease>,
    status: Option<u16>,
    finished: bool,
    pub upstream_success: bool,
}
impl Observation {
    pub fn new(metrics: Arc<RouteMetrics>) -> Self {
        metrics.active.inc();
        Self {
            metrics,
            started: Instant::now(),
            permit: None,
            lease: None,
            status: None,
            finished: false,
            upstream_success: true,
        }
    }
    pub fn status(&mut self, code: u16) {
        self.status = Some(code);
        self.metrics.status(code);
    }
    pub fn finish(&mut self, success: bool) {
        self.finished = true;
        if let Some(lease) = &mut self.lease {
            lease.finish(success && self.upstream_success);
        }
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        if self.status.is_none() {
            self.metrics.status(499);
        }
        if !self.finished {
            self.metrics.event("cancelled");
        }
        tracing::debug!(route = %self.metrics.label, status = self.status.unwrap_or(499), duration_seconds = self.started.elapsed().as_secs_f64(), completed = self.finished, "request finished");
        self.metrics.active.dec();
        self.metrics
            .duration
            .observe(self.started.elapsed().as_secs_f64());
    }
}

pub struct DeadlineBody {
    inner: Body,
    deadline: Pin<Box<Sleep>>,
    remaining: Option<usize>,
    observation: Option<Observation>,
    done: bool,
}
impl DeadlineBody {
    pub fn wrap(
        inner: Body,
        deadline: TokioInstant,
        remaining: Option<usize>,
        mut observation: Option<Observation>,
    ) -> Body {
        let done = inner.is_end_stream();
        if done {
            if let Some(obs) = &mut observation {
                obs.finish(true);
            }
            observation.take();
        }
        Body::new(Self {
            inner,
            deadline: Box::pin(tokio::time::sleep_until(deadline)),
            remaining,
            observation,
            done,
        })
    }
    fn complete(&mut self, success: bool, event: Option<&str>) {
        self.done = true;
        if let Some(mut obs) = self.observation.take() {
            if let Some(event) = event {
                obs.metrics.event(event);
            }
            obs.finish(success);
        }
    }
}
impl HttpBody for DeadlineBody {
    type Data = Bytes;
    type Error = BoxError;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        if this.deadline.as_mut().poll(cx).is_ready() {
            this.complete(false, Some("timeout"));
            return Poll::Ready(Some(Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "body deadline",
            )
            .into())));
        }
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(remaining) = &mut this.remaining {
                    let size = frame.data_ref().map_or(0, Bytes::len);
                    if size > *remaining {
                        this.complete(false, Some("body_limit"));
                        return Poll::Ready(Some(Err(std::io::Error::other(
                            "stream exceeds body limit",
                        )
                        .into())));
                    }
                    *remaining -= size;
                }
                if this.inner.is_end_stream() {
                    this.complete(true, None);
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(error))) => {
                this.complete(false, Some("stream_error"));
                Poll::Ready(Some(Err(error.into())))
            }
            Poll::Ready(None) => {
                this.complete(true, None);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.done
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
