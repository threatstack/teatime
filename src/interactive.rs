use std::io::{self,Write};

/// Prompt for text echoed in the terminal - _do not use for sensitive data_
pub fn interactive_text(prompt: &str) -> Result<String, io::Error> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    print!("{}", prompt);
    try!(stdout.flush());
    let mut line = String::new();
    let line_len = try!(stdin.read_line(&mut line));
    line.truncate(line_len - 1);

    Ok(line)
}
