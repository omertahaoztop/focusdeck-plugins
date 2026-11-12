use std::io;

use focusdeck_vikunja::i18n;
use focusdeck_vikunja::plugin::Vikunja;

fn main() -> io::Result<()> {
    i18n::init(focusdeck_i18n::requested(
        None,
        |k| std::env::var(k).ok(),
        &focusdeck_i18n::PLUGIN_ENV,
    ));
    focusdeck_protocol::serve(Vikunja::default(), io::stdin().lock(), io::stdout().lock())
}
