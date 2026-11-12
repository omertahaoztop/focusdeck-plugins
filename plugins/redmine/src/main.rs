use std::io;

use focusdeck_redmine::plugin::Redmine;

fn main() -> io::Result<()> {
    focusdeck_protocol::serve(Redmine::default(), io::stdin().lock(), io::stdout().lock())
}
