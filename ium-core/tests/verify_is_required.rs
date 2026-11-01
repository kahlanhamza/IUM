//! Compile-fail contract (issue #1, ADR-004): an adapter that does not
//! implement `verify` must NOT compile. Verify-before-declare is not opt-in.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/adapter_without_verify.rs");
}
