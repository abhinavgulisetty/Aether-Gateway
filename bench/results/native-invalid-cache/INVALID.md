# Invalid comparison — do not use for performance claims

This run was interrupted after detecting identical hashes for `original` and
`original-unlimited`. Extracting Git's old source timestamps into a reused Cargo
target directory allowed the relaxed-limiter artifact to be reused for the
unmodified target. It therefore did not measure the unmodified original gateway.

The harness now refreshes restored source modification times, asserts distinct
binary hashes, and exercises the original limiter before accepting measurements.
Raw partial measurements are retained for audit only. Use `../native-20261103`.
