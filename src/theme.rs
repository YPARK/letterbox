//! The screen's colours: sage grays, in steps from the text down to the
//! rules, with green for what is unread and orange for a flag; and muted
//! colours to tell accounts apart by, where their messages are together. `light` is
//! painted whatever the terminal's colours; `dark` keeps the terminal's own
//! background, as an Emacs buffer has it.

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, clap::ValueEnum)]
pub enum Name {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// The page, behind the messages and the one read.
    pub page: Color,
    /// The sidebar's, the top's and the foot's ground.
    pub side: Color,
    /// The search box.
    pub field: Color,
    /// Senders, and the subject of what is in focus.
    pub title: Color,
    /// What is read: the text of a message, an unread subject.
    pub text: Color,
    /// What is read less closely: subjects already read, folders, headers.
    pub soft: Color,
    /// What is glanced at: previews, dates, labels, counts, quoted lines.
    pub dim: Color,
    /// What is barely there: icons, signatures, hints.
    pub faint: Color,
    /// Lines between messages, and edges.
    pub rule: Color,
    /// The message or folder picked, in the pane with the focus; and in
    /// the others.
    pub focus: Color,
    pub row: Color,
    /// A message not yet read, and where the focus is.
    pub accent: Color,
    pub flag: Color,
    /// What went wrong, as an account that would not sign in.
    pub error: Color,
    /// The accounts', in turn.
    accounts: [Color; 4],
}

impl Theme {
    /// The account's colour: the accounts', in turn.
    pub fn account(&self, account: usize) -> Color {
        self.accounts[account % self.accounts.len()]
    }

    pub fn of(name: Name) -> Theme {
        match name {
            Name::Light => Theme {
                page: Color::Rgb(251, 252, 250),
                side: Color::Rgb(235, 239, 234),
                field: Color::Rgb(255, 255, 255),
                title: Color::Rgb(22, 26, 23),
                text: Color::Rgb(38, 43, 39),
                soft: Color::Rgb(86, 96, 88),
                dim: Color::Rgb(134, 145, 136),
                faint: Color::Rgb(176, 187, 178),
                rule: Color::Rgb(223, 229, 222),
                focus: Color::Rgb(206, 220, 205),
                row: Color::Rgb(227, 234, 226),
                accent: Color::Rgb(62, 128, 86),
                flag: Color::Rgb(196, 118, 40),
                error: Color::Rgb(176, 64, 58),
                accounts: [
                    Color::Rgb(58, 106, 156),
                    Color::Rgb(134, 78, 128),
                    Color::Rgb(36, 124, 124),
                    Color::Rgb(140, 112, 40),
                ],
            },
            Name::Dark => Theme {
                page: Color::Reset,
                side: Color::Rgb(32, 37, 33),
                field: Color::Rgb(44, 51, 46),
                title: Color::Rgb(238, 243, 238),
                text: Color::Rgb(214, 220, 214),
                soft: Color::Rgb(176, 186, 177),
                dim: Color::Rgb(132, 144, 134),
                faint: Color::Rgb(92, 104, 94),
                rule: Color::Rgb(48, 56, 50),
                focus: Color::Rgb(56, 72, 59),
                row: Color::Rgb(40, 48, 42),
                accent: Color::Rgb(122, 182, 136),
                flag: Color::Rgb(214, 170, 92),
                error: Color::Rgb(214, 112, 100),
                accounts: [
                    Color::Rgb(128, 168, 214),
                    Color::Rgb(198, 148, 192),
                    Color::Rgb(108, 188, 188),
                    Color::Rgb(206, 182, 108),
                ],
            },
        }
    }
}
