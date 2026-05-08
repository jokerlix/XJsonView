use std::process;

fn main() {
    let code = jfmt_cli::run_cli();
    process::exit(code.as_i32());
}
