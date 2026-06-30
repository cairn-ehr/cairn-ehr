//! Spike 0004 — the dense clinical form (operator-run, `--features gui`).
//!
//! This binary is the surface the operator drives for the passes CI cannot run:
//!
//! - `--dump-a11y` : print the **expected** accessibility tree (claim A1's
//!   checklist) as JSON and exit. Runs headless — no window — so it works on CI
//!   too; the live verdict is a screen-reader walk (see the crate README).
//! - `--latency`   : run the form on the **tiny-skia software renderer** (the
//!   Pi path) and log update→paint timings; feed them to
//!   [`iced_ui_spike::latency::summarize`] for claim L2.
//! - (no flag)     : run the form on the default renderer for the a11y / IME
//!   (claims A2 / I2 / I3) operator passes.
//!
//! Pre-1.0 caveat: this targets **iced 0.14**. iced breaks API between minor
//! releases (eval 0004 §4.3), so a future iced may need small touch-ups here.
//! That churn is exactly the fit-for-purpose risk the spike is measuring — it
//! lives only in this L3 harness and never reaches Cairn's core.
//!
//! The form never signs and never authors a real event: the UI is a pure L3
//! producer (ADR-0021). On "Save" it only prints the captured field values.

use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length};

use iced_ui_spike::corpus::corpus;
use iced_ui_spike::form::{clinical_form, to_json};

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();

    // Headless mode: dump the a11y contract and exit before touching a window.
    if args.iter().any(|a| a == "--dump-a11y") {
        print!("{}", to_json(&clinical_form()));
        return Ok(());
    }

    let software = args.iter().any(|a| a == "--latency");
    if software {
        // Force the tiny-skia software renderer — the Pi-class path with no GPU.
        // (Honoured by iced/wgpu's backend selection.)
        std::env::set_var("WGPU_BACKEND", "noop");
        std::env::set_var("ICED_BACKEND", "tiny-skia");
        eprintln!("[latency] software (tiny-skia) renderer requested; logging update→view timings");
    }

    // iced 0.14 takes a `boot` fn returning the initial state; the title is set
    // via the builder. (In 0.13 the first arg was the title — the kind of
    // pre-1.0 churn eval 0004 §4.3 flags.)
    iced::application(App::default, App::update, App::view)
        .title("Spike 0004 — clinical form")
        .run()
}

/// All form state in one struct — The Elm Architecture. Every field is visible
/// here and every mutation flows through `update`; this single-source-of-state
/// shape is the "stateful fit for a long-running clinical session" the eval
/// flags as iced's structural advantage.
struct App {
    identifier: String,
    /// The four multi-script name fields, indexed to match [`corpus`].
    names: Vec<String>,
    /// The medication list rows (mutable list — add/remove).
    meds: Vec<String>,
    /// Update→view timings (ms), collected only in `--latency` mode.
    latency_samples: Vec<f64>,
    latency_mode: bool,
}

impl Default for App {
    fn default() -> Self {
        // Pre-fill the name fields from the corpus so the operator immediately
        // sees Arabic RTL, Devanagari conjuncts, and Han glyphs without typing —
        // and can then test IME entry (I3) by editing the Han field.
        let names = corpus().into_iter().map(|s| s.text.to_string()).collect();
        let latency_mode = std::env::args().any(|a| a == "--latency");
        App {
            identifier: String::new(),
            names,
            meds: vec!["Amoxicillin 500 mg".into(), "Metformin 1 g".into()],
            latency_samples: Vec::new(),
            latency_mode,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Identifier(String),
    Name(usize, String),
    RemoveMed(usize),
    AddMed,
    Submit,
}

impl App {
    fn update(&mut self, message: Message) {
        // Stamp the start so `--latency` can measure update→view compute time, a
        // proxy for keystroke-to-paint. (True paint time needs a frame callback;
        // this captures the dominant in-process cost on weak hardware.)
        let started = std::time::Instant::now();

        match message {
            Message::Identifier(v) => self.identifier = v,
            Message::Name(i, v) => {
                if let Some(slot) = self.names.get_mut(i) {
                    *slot = v;
                }
            }
            Message::RemoveMed(i) => {
                if i < self.meds.len() {
                    self.meds.remove(i);
                }
            }
            Message::AddMed => self.meds.push(String::new()),
            Message::Submit => {
                // L3 never signs: just echo what was captured.
                println!("identifier = {}", self.identifier);
                for (i, n) in self.names.iter().enumerate() {
                    println!("name[{i}] = {n}");
                }
                for (i, m) in self.meds.iter().enumerate() {
                    println!("med[{i}] = {m}");
                }
            }
        }

        if self.latency_mode {
            self.latency_samples.push(started.elapsed().as_secs_f64() * 1000.0);
            // Print a rolling summary every 20 samples so the operator can read
            // the Pi result off the terminal without post-processing.
            if self.latency_samples.len() % 20 == 0 {
                if let Some(s) = iced_ui_spike::latency::summarize(&self.latency_samples) {
                    eprintln!(
                        "[latency] n={} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
                        s.n, s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms
                    );
                }
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        // Identifier field.
        let identifier = labelled(
            "Patient identifier",
            text_input("e.g. NHS / Medicare number", &self.identifier)
                .on_input(Message::Identifier)
                .into(),
        );

        // The four multi-script name fields, labelled by script.
        let labels = ["Name (Latin)", "Name (Arabic)", "Name (Devanagari)", "Name (Han / CJK)"];
        let mut names = column![].spacing(8);
        for (i, label) in labels.iter().enumerate() {
            let value = self.names.get(i).cloned().unwrap_or_default();
            names = names.push(labelled(
                label,
                text_input("", &value)
                    .on_input(move |v| Message::Name(i, v))
                    .into(),
            ));
        }

        // Medication list — mutable rows with a per-row Remove and a list Add.
        let mut meds = column![text("Medication list").size(18)].spacing(6);
        for (i, m) in self.meds.iter().enumerate() {
            meds = meds.push(
                row![
                    text(m.clone()).width(Length::Fill),
                    button("Remove").on_press(Message::RemoveMed(i)),
                ]
                .spacing(8),
            );
        }
        meds = meds.push(button("Add medication").on_press(Message::AddMed));

        let body = column![
            text("New patient — identity & medications").size(22),
            identifier,
            names,
            meds,
            button("Save patient record").on_press(Message::Submit),
        ]
        .spacing(16)
        .padding(20);

        container(scrollable(body))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// A label stacked above its control — keeps the accessible label adjacent to the
/// input so a screen reader announces them together (claim A1).
fn labelled<'a>(label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    column![text(label).size(14), control].spacing(4).into()
}
