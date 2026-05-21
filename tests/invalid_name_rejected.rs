mod common;

use common::TestEnv;

#[test]
fn name_with_single_quote_rejected_before_subprocess() {
    let env = TestEnv::new();
    env.bin().args(["new", "foo'bar"]).assert().failure();
}

#[test]
fn name_with_slash_rejected() {
    let env = TestEnv::new();
    env.bin().args(["new", "foo/bar"]).assert().failure();
}

#[test]
fn name_exactly_dotdot_rejected() {
    let env = TestEnv::new();
    env.bin().args(["new", ".."]).assert().failure();
}

#[test]
fn name_with_dollar_rejected() {
    let env = TestEnv::new();
    env.bin().args(["new", "foo$bar"]).assert().failure();
}
