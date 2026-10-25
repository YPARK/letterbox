//! The screen's colours: warm grays on paper, in steps from the text down
//! to the rules, with blue for what is unread and orange for a flag, which
//! the common colour blindnesses still tell apart; and colours to tell
//! accounts apart by, where their messages are together. Each text colour
//! is 4.5:1 or more on every ground it is drawn on, and the accounts'
//! colours keep clear of both. `light` is painted whatever the terminal's
//! colours; `dark` keeps the terminal's own background, as an Emacs buffer
//! has it.

use ratatui::style::Color;

/// What `dark`'s page, the terminal's own, is taken to be where a colour
/// is needed for it, as in a picture of the screen: the near-black most
/// terminals have, and their light gray text.
pub const TERMINAL_DARK: (u8, u8, u8) = (28, 28, 28);
pub const TERMINAL_DARK_TEXT: (u8, u8, u8) = (222, 222, 222);

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
    /// A label on the accent, as the button pressed with ⏎.
    pub on_accent: Color,
    pub flag: Color,
    /// What went wrong, as an account that would not sign in.
    pub error: Color,
    /// The accounts', in turn.
    accounts: [Color; 4],
}

impl Theme {
    /// The account's colour: the accounts', in turn.
    pub fn account(&self, account: usize) -> Color {
        let all = self.accounts();
        all[account % all.len()]
    }

    pub fn accounts(&self) -> &[Color] {
        &self.accounts
    }

    pub fn of(name: Name) -> Theme {
        match name {
            Name::Light => Theme {
                page: Color::Rgb(251, 248, 241),
                side: Color::Rgb(240, 234, 220),
                field: Color::Rgb(255, 253, 248),
                title: Color::Rgb(29, 26, 21),
                text: Color::Rgb(43, 39, 32),
                soft: Color::Rgb(64, 58, 49),
                dim: Color::Rgb(78, 71, 60),
                faint: Color::Rgb(90, 82, 70),
                rule: Color::Rgb(200, 188, 160),
                focus: Color::Rgb(216, 203, 166),
                row: Color::Rgb(230, 221, 198),
                accent: Color::Rgb(22, 86, 118),
                on_accent: Color::Rgb(255, 253, 248),
                flag: Color::Rgb(136, 64, 0),
                error: Color::Rgb(156, 40, 36),
                accounts: [
                    Color::Rgb(128, 58, 118),
                    Color::Rgb(22, 92, 74),
                    Color::Rgb(86, 72, 152),
                    Color::Rgb(80, 84, 16),
                ],
            },
            Name::Dark => Theme {
                page: Color::Reset,
                side: Color::Rgb(42, 39, 33),
                field: Color::Rgb(47, 43, 37),
                title: Color::Rgb(243, 238, 228),
                text: Color::Rgb(221, 214, 200),
                soft: Color::Rgb(196, 188, 172),
                dim: Color::Rgb(186, 177, 161),
                faint: Color::Rgb(178, 169, 153),
                rule: Color::Rgb(92, 84, 71),
                focus: Color::Rgb(68, 61, 50),
                row: Color::Rgb(54, 49, 42),
                accent: Color::Rgb(127, 182, 217),
                on_accent: Color::Rgb(24, 22, 18),
                flag: Color::Rgb(224, 160, 96),
                error: Color::Rgb(238, 150, 140),
                accounts: [
                    Color::Rgb(212, 156, 196),
                    Color::Rgb(124, 196, 174),
                    Color::Rgb(178, 166, 232),
                    Color::Rgb(178, 196, 110),
                ],
            },
        }
    }

    /// The page's colour, as a colour: `dark`'s, the terminal's own, as
    /// the near-black it is taken to be.
    pub fn ground(&self) -> Color {
        match self.page {
            Color::Reset => {
                let (r, g, b) = TERMINAL_DARK;
                Color::Rgb(r, g, b)
            }
            c => c,
        }
    }

    fn is_dark(&self) -> bool {
        self.page == Color::Reset
    }

    /// A hue on the map, in degrees, as a colour that reads on the page:
    /// each the same lightness and strength in OKLCH, so that no hue
    /// stands out but by itself; lighter on the dark page than the light.
    pub fn hue(&self, degrees: f32) -> Color {
        let (l, c) = if self.is_dark() {
            (0.80, 0.11)
        } else {
            (0.50, 0.12)
        };
        oklch(l, c, degrees)
    }
}

/// `a` and `b` mixed, `t` of the way from `a` to `b`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let t = t.clamp(0.0, 1.0);
            let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ if t < 0.5 => a,
        _ => b,
    }
}

/// A colour in OKLCH, Björn Ottosson's, as sRGB: lightness `l`, chroma `c`,
/// hue `h` in degrees.
fn oklch(l: f32, c: f32, h: f32) -> Color {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let linear = [
        4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3,
        -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3,
    ];
    let encode = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        let v = if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round() as u8
    };
    Color::Rgb(encode(linear[0]), encode(linear[1]), encode(linear[2]))
}
