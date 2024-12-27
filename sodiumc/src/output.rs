use std::{cmp, io};
use std::io::{stdout, Write};
use crossterm::{cursor, execute, queue, terminal};
use crossterm::event::KeyCode;
use crossterm::style::Attribute;
use crossterm::terminal::ClearType;
use crate::{prompt, CursorController, EditorContents, EditorRows, HighlightType, PythonHighlight, RustHighlight, SearchDirection, SearchIndex, StatusMessage, SyntaxHighlight, NAME, VERSION};
use crate::Reader;
use crate::KeyEvent;
use crate::KeyModifiers;

pub struct Output {
    pub(crate) win_size: (usize, usize),
    editor_contents: EditorContents,
    pub(crate) cursor_controller: CursorController,
    pub(crate) editor_rows: EditorRows,
    pub(crate) status_message: StatusMessage,
    pub(crate) dirty: u64,
    search_index: SearchIndex,
    pub(crate) syntax_highlight: Option<Box<dyn SyntaxHighlight>>,
}

impl Output {
    pub(crate) fn select_syntax(extension: &str) -> Option<Box<dyn SyntaxHighlight>> {
        let list: Vec<Box<dyn SyntaxHighlight>> = vec![Box::new(RustHighlight::new()), Box::new(PythonHighlight::new())];
        list.into_iter()
            .find(|it| it.extensions().contains(&extension))
    }

    pub(crate) fn new() -> Self {
        let win_size = terminal::size()
            .map(|(x, y)| (x as usize, y as usize - 2))
            .unwrap();
        let mut syntax_highlight = None; // modify
        Self {
            win_size,
            editor_contents: EditorContents::new(),
            cursor_controller: CursorController::new(win_size),
            editor_rows: EditorRows::new(&mut syntax_highlight), //modify
            status_message: StatusMessage::new(
                "HELP: Ctrl-S = Save | Ctrl-Q = Quit | Ctrl-F = Find".into(),
            ),
            dirty: 0,
            search_index: SearchIndex::new(),
            syntax_highlight,
        }
    }

    pub(crate) fn clear_screen() -> crossterm::Result<()> {
        execute!(stdout(), terminal::Clear(ClearType::All))?;
        execute!(stdout(), cursor::MoveTo(0, 0))
    }

    fn find_callback(output: &mut Output, keyword: &str, key_code: KeyCode) {
        if let Some((index, highlight)) = output.search_index.previous_highlight.take() {
            output.editor_rows.get_editor_row_mut(index).highlight = highlight;
        }
        match key_code {
            KeyCode::Esc | KeyCode::Enter => {
                output.search_index.reset();
            }
            _ => {
                output.search_index.y_direction = None;
                output.search_index.x_direction = None;
                match key_code {
                    KeyCode::Down => {
                        output.search_index.y_direction = SearchDirection::Forward.into()
                    }
                    KeyCode::Up => {
                        output.search_index.y_direction = SearchDirection::Backward.into()
                    }
                    KeyCode::Left => {
                        output.search_index.x_direction = SearchDirection::Backward.into()
                    }
                    KeyCode::Right => {
                        output.search_index.x_direction = SearchDirection::Forward.into()
                    }
                    _ => {}
                }
                for i in 0..output.editor_rows.number_of_rows() {
                    let row_index = match output.search_index.y_direction.as_ref() {
                        None => {
                            if output.search_index.x_direction.is_none() {
                                output.search_index.y_index = i;
                            }
                            output.search_index.y_index
                        }
                        Some(dir) => {
                            if matches!(dir, SearchDirection::Forward) {
                                output.search_index.y_index + i + 1
                            } else {
                                let res = output.search_index.y_index.saturating_sub(i);
                                if res == 0 {
                                    break;
                                }
                                res - 1
                            }
                        }
                    };
                    if row_index > output.editor_rows.number_of_rows() - 1 {
                        break;
                    }
                    let row = output.editor_rows.get_editor_row_mut(row_index);
                    let index = match output.search_index.x_direction.as_ref() {
                        None => row.render.find(&keyword),
                        Some(dir) => {
                            let index = if matches!(dir, SearchDirection::Forward) {
                                let start =
                                    cmp::min(row.render.len(), output.search_index.x_index + 1);
                                row.render[start..]
                                    .find(&keyword)
                                    .map(|index| index + start)
                            } else {
                                row.render[..output.search_index.x_index].rfind(&keyword)
                            };
                            if index.is_none() {
                                break;
                            }
                            index
                        }
                    };
                    if let Some(index) = index {
                        output.search_index.previous_highlight =
                            Some((row_index, row.highlight.clone()));
                        (index..index + keyword.len())
                            .for_each(|index| row.highlight[index] = HighlightType::SearchMatch);
                        output.cursor_controller.cursor_y = row_index;
                        output.search_index.y_index = row_index;
                        output.search_index.x_index = index;
                        output.cursor_controller.cursor_x = row.get_row_content_x(index);
                        output.cursor_controller.row_offset = output.editor_rows.number_of_rows();
                        break;
                    }
                }
            }
        }
    }

    pub(crate) fn find(&mut self) -> io::Result<()> {
        let cursor_controller = self.cursor_controller;
        if prompt!(
            self,
            "Search: {} (Use ESC / Arrows / Enter)",
            callback = Output::find_callback
        )
            .is_none()
        {
            self.cursor_controller = cursor_controller
        }
        Ok(())
    }

    fn draw_message_bar(&mut self) {
        queue!(
            self.editor_contents,
            terminal::Clear(ClearType::UntilNewLine)
        )
            .unwrap();
        if let Some(msg) = self.status_message.message() {
            self.editor_contents
                .push_str(&msg[..cmp::min(self.win_size.0, msg.len())]);
        }
    }

    pub(crate) fn delete_char(&mut self) {
        if self.cursor_controller.cursor_y == self.editor_rows.number_of_rows() {
            return;
        }
        if self.cursor_controller.cursor_y == 0 && self.cursor_controller.cursor_x == 0 {
            return;
        }
        if self.cursor_controller.cursor_x > 0 {
            self.editor_rows
                .get_editor_row_mut(self.cursor_controller.cursor_y)
                .delete_char(self.cursor_controller.cursor_x - 1);
            self.cursor_controller.cursor_x -= 1;
        } else {
            let previous_row_content = self
                .editor_rows
                .get_row(self.cursor_controller.cursor_y - 1);
            self.cursor_controller.cursor_x = previous_row_content.len();
            self.editor_rows
                .join_adjacent_rows(self.cursor_controller.cursor_y);
            self.cursor_controller.cursor_y -= 1;
        }
        if let Some(it) = self.syntax_highlight.as_ref() {
            it.update_syntax(
                self.cursor_controller.cursor_y,
                &mut self.editor_rows.row_contents,
            );
        }
        self.dirty += 1;
    }

    pub(crate) fn insert_newline(&mut self) {
        if self.cursor_controller.cursor_x == 0 {
            self.editor_rows
                .insert_row(self.cursor_controller.cursor_y, String::new())
        } else {
            let current_row = self
                .editor_rows
                .get_editor_row_mut(self.cursor_controller.cursor_y);
            let new_row_content = current_row.row_content[self.cursor_controller.cursor_x..].into();
            current_row
                .row_content
                .truncate(self.cursor_controller.cursor_x);
            EditorRows::render_row(current_row);
            self.editor_rows
                .insert_row(self.cursor_controller.cursor_y + 1, new_row_content);
            if let Some(it) = self.syntax_highlight.as_ref() {
                it.update_syntax(
                    self.cursor_controller.cursor_y,
                    &mut self.editor_rows.row_contents,
                );
                it.update_syntax(
                    self.cursor_controller.cursor_y + 1,
                    &mut self.editor_rows.row_contents,
                )
            }
        }
        self.cursor_controller.cursor_x = 0;
        self.cursor_controller.cursor_y += 1;
        self.dirty += 1;
    }

    pub(crate) fn insert_char(&mut self, ch: char) {
        if self.cursor_controller.cursor_y == self.editor_rows.number_of_rows() {
            self.editor_rows
                .insert_row(self.editor_rows.number_of_rows(), String::new());
            self.dirty += 1;
        }
        self.editor_rows
            .get_editor_row_mut(self.cursor_controller.cursor_y)
            .insert_char(self.cursor_controller.cursor_x, ch);
        if let Some(it) = self.syntax_highlight.as_ref() {
            it.update_syntax(
                self.cursor_controller.cursor_y,
                &mut self.editor_rows.row_contents,
            )
        }
        self.cursor_controller.cursor_x += 1;
        self.dirty += 1;
    }

    fn draw_status_bar(&mut self) {
        self.editor_contents
            .push_str(&Attribute::Reverse.to_string());
        let info = format!(
            "{} {} -- {} lines",
            self.editor_rows
                .filename
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("[No Name]"),
            if self.dirty > 0 { "(modified)" } else { "" },
            self.editor_rows.number_of_rows()
        );
        let info_len = cmp::min(info.len(), self.win_size.0);
        /* modify the following */
        let line_info = format!(
            "{} | {}/{}",
            self.syntax_highlight
                .as_ref()
                .map(|highlight| highlight.file_type())
                .unwrap_or("no ft"),
            self.cursor_controller.cursor_y + 1,
            self.editor_rows.number_of_rows()
        );
        self.editor_contents.push_str(&info[..info_len]);
        for i in info_len..self.win_size.0 {
            if self.win_size.0 - i == line_info.len() {
                self.editor_contents.push_str(&line_info);
                break;
            } else {
                self.editor_contents.push(' ')
            }
        }
        self.editor_contents
            .push_str(&Attribute::Reset.to_string());
        self.editor_contents.push_str("\r\n");
    }

    fn draw_rows(&mut self) {
        let screen_rows = self.win_size.1;
        let screen_columns = self.win_size.0;
        for i in 0..screen_rows {
            let file_row = i + self.cursor_controller.row_offset;
            if file_row >= self.editor_rows.number_of_rows() {
                if self.editor_rows.number_of_rows() == 0 && i == screen_rows / 3 {
                    let mut welcome = format!("{} --- Version {}", NAME, VERSION);
                    if welcome.len() > screen_columns {
                        welcome.truncate(screen_columns)
                    }
                    let mut padding = (screen_columns - welcome.len()) / 2;
                    if padding != 0 {
                        self.editor_contents.push('~');
                        padding -= 1
                    }
                    (0..padding).for_each(|_| self.editor_contents.push(' '));
                    self.editor_contents.push_str(&welcome);
                } else {
                    self.editor_contents.push('~');
                }
            } else {
                let row = self.editor_rows.get_editor_row(file_row);
                let render = &row.render;
                let column_offset = self.cursor_controller.column_offset;
                let len = cmp::min(render.len().saturating_sub(column_offset), screen_columns);
                let start = if len == 0 { 0 } else { column_offset };
                let render = render.chars().skip(start).take(len).collect::<String>();
                self.syntax_highlight
                    .as_ref()
                    .map(|syntax_highlight| {
                        syntax_highlight.color_row(
                            &render,
                            &row.highlight[start..start + len],
                            &mut self.editor_contents,
                        )
                    })
                    .unwrap_or_else(|| self.editor_contents.push_str(&render));
            }
            queue!(
                self.editor_contents,
                terminal::Clear(ClearType::UntilNewLine)
            )
                .unwrap();
            self.editor_contents.push_str("\r\n");
        }
    }

    pub(crate) fn move_cursor(&mut self, direction: KeyCode) {
        self.cursor_controller
            .move_cursor(direction, &self.editor_rows);
    }

    pub(crate) fn refresh_screen(&mut self) -> crossterm::Result<()> {
        self.cursor_controller.scroll(&self.editor_rows);
        queue!(self.editor_contents, cursor::Hide, cursor::MoveTo(0, 0))?;
        self.draw_rows();
        self.draw_status_bar();
        self.draw_message_bar();
        let cursor_x = self.cursor_controller.render_x - self.cursor_controller.column_offset;
        let cursor_y = self.cursor_controller.cursor_y - self.cursor_controller.row_offset;
        queue!(
            self.editor_contents,
            cursor::MoveTo(cursor_x as u16, cursor_y as u16),
            cursor::Show
        )?;
        self.editor_contents.flush()
    }
}