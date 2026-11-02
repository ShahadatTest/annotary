fn main() {
    // Behave like other Unix tools when stdout is closed early (`annotary … | head`): die
    // quietly on SIGPIPE instead of panicking with "failed printing to stdout".
    #[cfg(unix)]
    // SAFETY: resetting a signal disposition before any other thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    std::process::exit(annotary::cli::run(std::env::args_os()));
}
