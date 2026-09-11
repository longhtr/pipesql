//! Narrow caller around unchanged CLI source; native rename refusal is external.
#[allow(dead_code)]
mod cli {
    include!("../../src/cli/mod.rs");
    pub(super) fn entry() -> std::process::ExitCode {
        main()
    }
}

unsafe extern "C" {
    fn cli_publication_start(position: u32);
    fn cli_publication_calls() -> u32;
}

fn main() -> std::process::ExitCode {
    let position = std::env::var("PIPESQL_RENAME_CUT")
        .unwrap()
        .parse::<u32>()
        .unwrap();
    assert!(position <= 4);
    // SAFETY: the linked observer is configured before this single-threaded CLI
    // entry. Both calls exchange integers only; the observer retains no pointer.
    unsafe { cli_publication_start(position) };
    let outcome = cli::entry();
    let calls = unsafe { cli_publication_calls() };
    eprintln!("observed_renames={calls}");
    outcome
}
