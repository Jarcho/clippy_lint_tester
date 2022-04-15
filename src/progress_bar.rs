// Pretty bad progress bar.
// The progress bar is redraw using the old trick of writing \r and blanking out the
// previous display with spaces.

use atty;
use std::fmt::{self, Display};
use std::time::{Duration, Instant};

struct ProgressBarState {
    start: Instant,
    current: usize,
    total: usize,
    displayed: bool,
}

const WIDTH: usize = 70;

impl ProgressBarState {
    pub fn new(total: usize) -> ProgressBarState {
        ProgressBarState {
            start: Instant::now(),
            current: 0,
            total,
            displayed: false,
        }
    }

    pub fn inc(&mut self, message: &str) {
        self.current += 1;
        self.redraw(message);
    }

    pub fn redraw(&mut self, message: &str) {
        self.displayed = true;

        let total: u128 = self.total.try_into().expect("total fits u64");
        let current: u128 = self.current.try_into().expect("current <= total");
        let elapsed_ms = Instant::now().duration_since(self.start).as_millis();
        let ms_per_task = (current > 1).then(|| elapsed_ms / current);
        let eta_ms = ms_per_task.map(|x| (total - current) * x);
        let eta_duration =
            eta_ms.map(|x| Duration::from_millis(x.try_into().expect("fits into u64")));

        let display = self.display(message, eta_duration, WIDTH);
        eprint!("\r{}", display);
    }

    fn display<'a>(
        &self,
        message: &'a str,
        eta: Option<Duration>,
        width: usize,
    ) -> ProgressBarDisplay<'a> {
        ProgressBarDisplay {
            current: self.current,
            total: self.total,
            message,
            eta,
            width,
        }
    }
}

impl Drop for ProgressBarState {
    fn drop(&mut self) {
        if self.displayed {
            eprint!("\r{0:1$}\r", "", WIDTH);
        }
    }
}

fn num_digits(mut value: usize) -> usize {
    let mut result = 0;
    while value > 0 {
        value /= 10;
        result += 1;
    }
    result
}

struct ProgressBarDisplay<'a> {
    current: usize,
    total: usize,
    message: &'a str,
    eta: Option<Duration>,
    width: usize,
}

impl fmt::Display for ProgressBarDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const PROGRESS_LENGTH: usize = 20;

        let total_width = num_digits(self.total);

        let message_width = self.width - (2 * total_width + 3) - (PROGRESS_LENGTH + 5) - 4;

        let message = self.message.get(0..message_width).unwrap_or(self.message);
        let progress_filled = PROGRESS_LENGTH * self.current / self.total;
        let progress_remaining = PROGRESS_LENGTH - progress_filled;

        write!(
            f,
            "{0:2$}/{1}: {3:4$} [{5:=<6$}>{7: <8$}] ",
            self.current,
            self.total,
            total_width,
            message,
            message_width,
            "",
            progress_filled,
            "",
            progress_remaining
        )?;

        let secs = self.eta.map(|x| x.as_secs());
        let mins = secs.map(|x| (x + 30) / 60);
        match mins {
            None => write!(f, "   ")?,
            Some(0) => write!(f, " <1m")?,
            Some(x) if x > 999 => write!(f, "!!!m")?,
            Some(x) => write!(f, "{:3}m", x)?,
        };

        Ok(())
    }
}

#[derive(Default)]
pub struct ProgressBar {
    stdout_is_tty: bool,
    stderr_is_tty: bool,
    progress_bar: Option<ProgressBarState>,
}

impl ProgressBar {
    #[must_use]
    pub fn new() -> Self {
        ProgressBar {
            stdout_is_tty: atty::is(atty::Stream::Stdout),
            stderr_is_tty: atty::is(atty::Stream::Stderr),
            progress_bar: None,
        }
    }

    pub fn display_progress(&mut self, total: usize, message: &str) {
        if !self.stderr_is_tty {
            return;
        }

        let mut progress_bar = ProgressBarState::new(total);
        progress_bar.redraw(message);
        self.progress_bar = Some(progress_bar);
    }

    pub fn inc_progress(&mut self, message: &str) {
        if let Some(progress_bar) = &mut self.progress_bar {
            progress_bar.inc(message);
        }
    }

    pub fn println(&mut self, progress_message: &str, message: impl Display) {
        if let Some(progress_bar) = &mut self.progress_bar {
            if self.stdout_is_tty {
                println!("\r{0:1$}\r{2}", "", WIDTH, message);
                progress_bar.redraw(progress_message);
                return;
            }
        }

        println!("{}", message);
    }

    pub fn eprintln(&mut self, progress_message: &str, message: impl Display) {
        if let Some(progress_bar) = &mut self.progress_bar {
            eprintln!("\r{0:1$}\r{2}", "", WIDTH, message);
            progress_bar.redraw(progress_message);
            return;
        }
        eprintln!("{}", message);
    }
}

#[cfg(test)]
mod test {
    use super::ProgressBarDisplay;

    use expect_test::expect;
    use std::time::Duration;

    #[test]
    fn progress_bar_display() {
        let bar_display = ProgressBarDisplay {
            current: 30,
            total: 100,
            message: "message",
            eta: Some(Duration::from_secs(123)),
            width: 80,
        };
        let expected = expect![[
            r#" 30/100: message                                    [======>              ]   2m"#
        ]];
        expected.assert_eq(&format!("{}", bar_display));
    }
}
