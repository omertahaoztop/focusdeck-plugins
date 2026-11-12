use std::io;

use focusdeck_whatsapp::i18n;
use focusdeck_whatsapp::plugin::WhatsApp;

fn main() -> io::Result<()> {
    i18n::init(focusdeck_i18n::requested(
        None,
        |k| std::env::var(k).ok(),
        &focusdeck_i18n::PLUGIN_ENV,
    ));
    focusdeck_protocol::serve(WhatsApp::default(), io::stdin().lock(), io::stdout().lock())
}
