//! Stamps the commit this exe was built from, exactly as the `benilla` launcher shim does
//! (`benilla-buildstamp` owns the rule).

fn main() {
    benilla_buildstamp::emit();
}
