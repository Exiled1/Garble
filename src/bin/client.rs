use anyhow::Result;
use crossterm::{cursor, event, execute, queue, style, terminal};
use futures::{Stream, StreamExt};
use pin_project::{pin_project, pinned_drop};
use std::{
    env,
    io::{self, Write},
    task::Poll,
};
use tokio::select;

use garble::client::ChatClient;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    println!("{:?}", args);
    let host: String;
    let port: Option<usize>;

    if args.len() == 2 || args.len() == 3 {
        host = args[1].to_owned();
        port = args
            .get(2)
            .and_then(|a| a.to_owned().parse::<usize>().ok())
            .filter(|b| *b <= 65535);
    } else {
        eprintln!("Error: wrong number of arguments. Requires hostname and optional port number");
        std::process::exit(1);
    }
    let port = port.unwrap_or(3000);

    let mut chat_client = ChatClient::new(port, host).await?; // make a new client

    println!("Connected!");
    let mut readline = Readline::new("> ".to_string()).fuse();

    loop {
        select! {
            msg = readline.next() => {
                match msg {
                    Some(msg) => readline.get_mut().println(format_args!("> {msg}")),
                    None => break
                }
            }
            msg = chat_client.terminal_task.inbound.recv() => {
                match msg {
                    Some(msg) => readline.get_mut().println(format_args!("< {msg}")),
                    None => readline.get_mut().println(format_args!("Connection closed."))
                }
            }
        }
    }

    Ok(())
}

/// A stream that allows the user to enter lines of input while simultaneously displaying recieved
/// messages in a separate "window".
/// TODO: line wrapping is not implemented
#[pin_project(PinnedDrop)]
#[derive(Default)]
struct Readline {
    /// The terminal input stream.
    #[pin]
    events: event::EventStream,

    /// The buffered contents of the line so far.
    current_line: String,

    /// The prompt to display before the user input.
    prompt: String,

    /// Whether to stay in raw mode after the input reader is dropped.
    prev_raw_mode_enabled: bool,

    term_rows: u16,
    cursor_row: u16,
}

impl Readline {
    fn new(prompt: String) -> Self {
        let prev_raw_mode_enabled = terminal::is_raw_mode_enabled().unwrap_or(false);
        terminal::enable_raw_mode().expect("I/O error writing stdout");

        let mut result = Self {
            events: event::EventStream::new(),
            current_line: String::new(),
            prompt,
            prev_raw_mode_enabled,
            term_rows: terminal::size().expect("I/O error writing stdout").1,
            cursor_row: cursor::position().expect("I/O error writing stdout").1,
        };

        // Maintain one blank line between the output window and the input line.
        execute!(
            io::stdout(),
            terminal::Clear(terminal::ClearType::CurrentLine),
            result.scroll_if_needed(),
            cursor::MoveToNextLine(1),
            terminal::Clear(terminal::ClearType::CurrentLine),
            style::Print(&result.prompt)
        )
        .expect("I/O error writing stdout");

        result
    }
}

#[pinned_drop]
impl PinnedDrop for Readline {
    fn drop(self: std::pin::Pin<&mut Self>) {
        if !self.prev_raw_mode_enabled {
            terminal::disable_raw_mode().expect("I/O error writing stdout");
            println!();
        }
    }
}

impl Readline {
    fn println(&mut self, line: impl std::fmt::Display) {
        self.println_fmt(format_args!("{}", line));
    }
    fn println_fmt(&mut self, line: std::fmt::Arguments<'_>) {
        let mut stdout = io::stdout();
        queue!(
            stdout,
            terminal::Clear(terminal::ClearType::CurrentLine),
            cursor::MoveToPreviousLine(1),
            style::Print(format_args!("{}", line)),
            self.scroll_if_needed(),
            cursor::MoveToNextLine(2),
            terminal::Clear(terminal::ClearType::CurrentLine),
            style::Print(&self.prompt),
            style::Print(&self.current_line)
        )
        .expect("I/O error writing stdout");
    }

    fn scroll_if_needed(&mut self) -> impl crossterm::Command {
        if self.cursor_row == (self.term_rows - 1) {
            terminal::ScrollUp(1)
        } else {
            self.cursor_row += 1;
            terminal::ScrollUp(0)
        }
    }
}

impl Stream for Readline {
    type Item = String;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut stdout = io::stdout();

        // For each available event...
        while let Poll::Ready(event) = self.as_mut().project().events.poll_next(cx) {
            // What kind of event is it?
            let event = match event {
                // It's a key event, handle it below.
                Some(Ok(event::Event::Key(e))) => e,

                // Some event we don't care about.
                Some(Ok(_)) => continue,

                // The terminal has been closed; return None to indicate the end of the stream.
                None => return Poll::Ready(None),

                // Something went wrong, let's bail out.
                Some(Err(e)) => panic!("I/O error reading stdin: {e:?}"),
            };

            // It's a keypress.
            // Any modifiers (ignoring shift)?
            match event.modifiers.difference(event::KeyModifiers::SHIFT) {
                // No.
                event::KeyModifiers::NONE => match event.code {
                    // The user entered a character; add it to the buffer
                    event::KeyCode::Char(c) => {
                        self.current_line.push(c);
                        // Echo it.
                        queue!(stdout, style::Print(c)).expect("I/O error writing stdout");
                    }

                    // The user pressed backspace; delete the last entered character.
                    event::KeyCode::Backspace => {
                        if self.current_line.pop().is_some() {
                            queue!(
                                stdout,
                                terminal::Clear(terminal::ClearType::CurrentLine),
                                cursor::MoveToColumn(1),
                                style::Print(&self.prompt),
                                style::Print(&self.current_line)
                            )
                            .expect("I/O error writing stdout");
                        } else {
                            queue!(stdout, style::Print('\u{07}'))
                                .expect("I/O error writing stdout"); // beep
                        }
                    }

                    // The user pressed enter; consume & yield the buffer.
                    event::KeyCode::Enter => {
                        execute!(
                            stdout,
                            terminal::Clear(terminal::ClearType::CurrentLine),
                            cursor::MoveToColumn(1),
                            style::Print(&self.prompt)
                        )
                        .expect("I/O error writing stdout");
                        return Poll::Ready(Some(std::mem::take(&mut self.current_line)));
                    }
                    _ => {}
                },

                // Yes -- control. Ctrl+C or Ctrl+D?
                event::KeyModifiers::CONTROL => match event.code {
                    // Ctrl+C: kill the current line
                    event::KeyCode::Char('c') => {
                        self.current_line.clear();
                        queue!(
                            stdout,
                            terminal::Clear(terminal::ClearType::CurrentLine),
                            cursor::MoveToColumn(1),
                            style::Print(&self.prompt)
                        )
                        .expect("I/O error writing to stdout");
                    }

                    // Ctrl+D: disconnect
                    event::KeyCode::Char('d') => {
                        execute!(stdout, style::Print("^D")).expect("I/O error writing to stdout");
                        return Poll::Ready(None);
                    }

                    // Neither, wait for the next event.
                    _ => {}
                },

                // Yes, something else -- ignore it and wait for the next event.
                _ => {}
            }
        }

        // No events are available, ask the caller to wait for the next one.
        stdout.flush().expect("I/O error writing stdout");
        Poll::Pending
    }
}
