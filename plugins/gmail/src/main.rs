use std::io;
use std::process::ExitCode;

use seenboard_gmail::i18n;
use seenboard_gmail::login::{self, LoginArgs};
use seenboard_gmail::plugin::Gmail;

fn main() -> io::Result<ExitCode> {
    i18n::init(seenboard_i18n::requested(
        None,
        |k| std::env::var(k).ok(),
        &seenboard_i18n::PLUGIN_ENV,
    ));
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("login") {
        return Ok(match LoginArgs::parse(args).and_then(login::run) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        });
    }
    seenboard_protocol::serve(Gmail::default(), io::stdin().lock(), io::stdout().lock())?;
    Ok(ExitCode::SUCCESS)
}
