use std::io;

use seenboard_whatsapp::i18n;
use seenboard_whatsapp::plugin::WhatsApp;

fn main() -> io::Result<()> {
    i18n::init(seenboard_i18n::requested(
        None,
        |k| std::env::var(k).ok(),
        &seenboard_i18n::PLUGIN_ENV,
    ));
    seenboard_protocol::serve(WhatsApp::default(), io::stdin().lock(), io::stdout().lock())
}
