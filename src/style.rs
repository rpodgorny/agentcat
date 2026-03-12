use crossterm::style::{Attribute, Color, SetAttribute, SetForegroundColor, ResetColor};
use std::io::{self, Write};

pub struct Style {
    pub use_emoji: bool,
    pub use_color: bool,
}

impl Style {
    pub fn new(use_emoji: bool, use_color: bool) -> Self {
        Self { use_emoji, use_color }
    }

    pub fn icon_session(&self) -> &str {
        if self.use_emoji { "\u{1f431}" } else { "[agentcat]" }
    }

    pub fn icon_tool(&self) -> &str {
        if self.use_emoji { "\u{1f527}" } else { "[tool]" }
    }

    pub fn icon_ok(&self) -> &str {
        if self.use_emoji { "\u{2705}" } else { "[ok]" }
    }

    pub fn icon_err(&self) -> &str {
        if self.use_emoji { "\u{274c}" } else { "[err]" }
    }

    pub fn icon_think(&self) -> &str {
        if self.use_emoji { "\u{1f4ad}" } else { "[think]" }
    }

    pub fn icon_warn(&self) -> &str {
        if self.use_emoji { "\u{26a0}\u{fe0f}" } else { "[warn]" }
    }

    pub fn icon_compact(&self) -> &str {
        if self.use_emoji { "\u{1f4e6}" } else { "[compact]" }
    }

    pub fn icon_done(&self) -> &str {
        if self.use_emoji { "\u{2728}" } else { "[done]" }
    }

    pub fn icon_cost(&self) -> &str {
        if self.use_emoji { "\u{1f4b0}" } else { "[cost]" }
    }

    pub fn icon_stats(&self) -> &str {
        if self.use_emoji { "\u{1f4ca}" } else { "[stats]" }
    }

    pub fn write_bold_cyan(&self, w: &mut impl Write, text: &str) -> io::Result<()> {
        if self.use_color {
            write!(w, "{}{}{}{}", SetAttribute(Attribute::Bold), SetForegroundColor(Color::Cyan), text, ResetColor)?;
            write!(w, "{}", SetAttribute(Attribute::Reset))?;
        } else {
            write!(w, "{}", text)?;
        }
        Ok(())
    }

    pub fn write_cyan(&self, w: &mut impl Write, text: &str) -> io::Result<()> {
        if self.use_color {
            write!(w, "{}{}{}", SetForegroundColor(Color::Cyan), text, ResetColor)?;
        } else {
            write!(w, "{}", text)?;
        }
        Ok(())
    }

    pub fn write_green(&self, w: &mut impl Write, text: &str) -> io::Result<()> {
        if self.use_color {
            write!(w, "{}{}{}", SetForegroundColor(Color::Green), text, ResetColor)?;
        } else {
            write!(w, "{}", text)?;
        }
        Ok(())
    }

    pub fn write_red(&self, w: &mut impl Write, text: &str) -> io::Result<()> {
        if self.use_color {
            write!(w, "{}{}{}", SetForegroundColor(Color::Red), text, ResetColor)?;
        } else {
            write!(w, "{}", text)?;
        }
        Ok(())
    }

    pub fn write_dim(&self, w: &mut impl Write, text: &str) -> io::Result<()> {
        if self.use_color {
            write!(w, "{}{}{}", SetAttribute(Attribute::Dim), text, SetAttribute(Attribute::Reset))?;
        } else {
            write!(w, "{}", text)?;
        }
        Ok(())
    }

    pub fn write_yellow(&self, w: &mut impl Write, text: &str) -> io::Result<()> {
        if self.use_color {
            write!(w, "{}{}{}", SetForegroundColor(Color::Yellow), text, ResetColor)?;
        } else {
            write!(w, "{}", text)?;
        }
        Ok(())
    }

    pub fn write_bold(&self, w: &mut impl Write, text: &str) -> io::Result<()> {
        if self.use_color {
            write!(w, "{}{}{}", SetAttribute(Attribute::Bold), text, SetAttribute(Attribute::Reset))?;
        } else {
            write!(w, "{}", text)?;
        }
        Ok(())
    }
}
