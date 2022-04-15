use std::borrow::Cow;
use std::fmt::Display;
use std::io::Write;
use std::iter::IntoIterator;
use std::ops::Deref;

use anyhow::Result;
use unicode_segmentation::UnicodeSegmentation;

pub enum Alignment {
    Left,
    Center,
    Right,
}

pub trait TableDisplay: Display {
    fn display_width(&self) -> usize;
    #[must_use]
    fn alignment() -> Alignment {
        Alignment::Left
    }
}

impl TableDisplay for &str {
    fn display_width(&self) -> usize {
        self.graphemes(true).count()
    }
}

impl TableDisplay for usize {
    fn display_width(&self) -> usize {
        let mut outcome = 0;
        let mut value = *self;

        loop {
            outcome += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }

        outcome += (outcome - 1) / 3;

        outcome
    }

    fn alignment() -> Alignment {
        Alignment::Right
    }
}

impl<'a> TableDisplay for Cow<'a, str> {
    fn display_width(&self) -> usize {
        self.deref().display_width()
    }
}

impl<T> TableDisplay for &T
where
    T: TableDisplay,
{
    fn display_width(&self) -> usize {
        (*self).display_width()
    }

    fn alignment() -> Alignment {
        T::alignment()
    }
}

pub fn print_table<A, B>(
    headers: [&str; 2],
    data: impl IntoIterator<Item = (A, B)> + Copy,
    mut output: impl Write,
) -> Result<()>
where
    A: TableDisplay,
    B: TableDisplay,
{
    let widths: [usize; 2] = data.into_iter().fold(
        [headers[0].display_width(), headers[1].display_width()],
        |widths, (a, b)| {
            [
                widths[0].max(a.display_width()),
                widths[1].max(b.display_width()),
            ]
        },
    );

    match A::alignment() {
        Alignment::Left => write!(output, " {0:<1$} ", headers[0], widths[0])?,
        Alignment::Center => write!(output, " {0:^1$} ", headers[0], widths[0])?,
        Alignment::Right => write!(output, " {0:>1$} ", headers[0], widths[0])?,
    }

    write!(output, "|")?;

    match B::alignment() {
        Alignment::Left => writeln!(output, " {0:<1$} ", headers[1], widths[1])?,
        Alignment::Center => writeln!(output, " {0:^1$} ", headers[1], widths[1])?,
        Alignment::Right => writeln!(output, " {0:>1$} ", headers[1], widths[1])?,
    }

    match A::alignment() {
        Alignment::Left => write!(output, ":{0:-^1$}", "", widths[0] + 1)?,
        Alignment::Center => write!(output, ":{0:-^1$}:", "", widths[0] - 1)?,
        Alignment::Right => write!(output, "{0:-^1$}:", "", widths[0] + 1)?,
    }

    write!(output, "|")?;

    match B::alignment() {
        Alignment::Left => writeln!(output, ":{0:-^1$}", "", widths[1] + 1)?,
        Alignment::Center => writeln!(output, ":{0:-^1$}:", "", widths[1] - 1)?,
        Alignment::Right => writeln!(output, "{0:-^1$}:", "", widths[1] + 1)?,
    }

    for (a, b) in data {
        writeln!(output, " {0:1$} | {2:3$} ", a, widths[0], b, widths[1])?;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use expect_test::expect;

    use super::*;

    #[test]
    fn print_single_column() {
        let mut v = vec![];
        print_table(["A", "B"], [("a", 1), ("bb", 22), ("ccc", 333)], &mut v).unwrap();
        let s = String::from_utf8(v).unwrap();
        let expected = expect![[r#"
             A   |   B 
            :----|----:
             a   |   1 
             bb  |  22 
             ccc | 333 
        "#]];
        expected.assert_eq(&s);
    }
}
