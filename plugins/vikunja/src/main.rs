use std::io;

use focusdeck_vikunja::plugin::Vikunja;

fn main() -> io::Result<()> {
    focusdeck_protocol::serve(Vikunja::default(), io::stdin().lock(), io::stdout().lock())
}
