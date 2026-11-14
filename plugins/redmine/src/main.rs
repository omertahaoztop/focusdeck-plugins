use std::io;

use seenboard_redmine::i18n;
use seenboard_redmine::plugin::Redmine;

fn main() -> io::Result<()> {
    i18n::init(seenboard_i18n::requested(
        None,
        |k| std::env::var(k).ok(),
        &seenboard_i18n::PLUGIN_ENV,
    ));
    seenboard_protocol::serve(Redmine::default(), io::stdin().lock(), io::stdout().lock())
}
