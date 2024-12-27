use std::time::Duration;
use crossterm::event::{poll, read, Event, KeyEvent};

pub struct Reader;

impl Reader {
    pub fn read_key(&self) -> crossterm::Result<KeyEvent> {
        loop {
            if poll(Duration::from_millis(500))? {
                if let Event::Key(event) = read()? {
                    return Ok(event);
                }
            }
        }
    }
}