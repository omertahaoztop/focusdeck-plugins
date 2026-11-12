use std::io;

use focusdeck_zulip::plugin::Zulip;

fn main() -> io::Result<()> {
    focusdeck_protocol::serve(Zulip::default(), io::stdin().lock(), io::stdout().lock())
}
