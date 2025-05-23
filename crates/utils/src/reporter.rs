use crate::{Error, GPosIdx, GlobalPositionTable};
use codespan_reporting::term::termcolor::ColorChoice;
use codespan_reporting::{
    diagnostic::{Diagnostic, Label, LabelStyle},
    term::{self, termcolor::StandardStream},
};
use std::collections::BTreeMap;

#[derive(PartialOrd, Ord, Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// Index for information associated with a [Diagnostic] instance.
// XXX: Define `add_message` and `add_info` on this type so that user code can use it as a builder pattern.
pub struct InfoIdx(usize);

#[derive(Eq, PartialEq)]
/// Information attached to a [Diagnostic] instance.
// XXX: Change this to track a vector of messages and positions which logically
// correspond to one error.
struct Information {
    message: String,
    pos: GPosIdx,
}

impl Information {
    fn new(message: String, pos: GPosIdx) -> Self {
        Self { message, pos }
    }
}

#[derive(Default)]
/// Track diagnostics that need to be reported
pub struct Diagnostics {
    /// Information being tracked by this instance.
    infos: Vec<Information>,
    /// Errors that have been reported.
    errors: Vec<Error>,
}

impl Diagnostics {
    fn add_info_help(&mut self, info: Information) -> InfoIdx {
        if let Some(idx) = self.infos.iter().position(|i| *i == info) {
            InfoIdx(idx)
        } else {
            let idx = self.infos.len();
            self.infos.push(info);
            InfoIdx(idx)
        }
    }

    /// Add information to the diagnostics instance.
    pub fn add_info<S: ToString>(
        &mut self,
        message: S,
        pos: GPosIdx,
    ) -> InfoIdx {
        if pos == GPosIdx::UNKNOWN {
            log::trace!(
                "Adding info with unknown position: {}",
                message.to_string()
            )
        }
        self.add_info_help(Information::new(message.to_string(), pos))
    }

    /// Add a message without any position information.
    pub fn add_message<S: ToString>(&mut self, message: S) -> InfoIdx {
        self.add_info_help(Information::new(
            message.to_string(),
            GPosIdx::UNKNOWN,
        ))
    }

    /// Add an error to the diagnostics instance.
    // XXX: Make this add a new information object so that its easy to express
    // the "create error and add info" pattern.
    pub fn add_error(&mut self, error: Error) {
        if !self.errors.contains(&error) {
            log::trace!("Adding error: {}", error.kind);
            self.errors.push(error);
        }
    }

    /// Report all errors and return the number of errors.
    /// Returns None if there are no errors.
    pub fn report_all(&mut self) -> Option<u64> {
        let is_tty = atty::is(atty::Stream::Stderr);
        let writer = StandardStream::stderr(if is_tty {
            ColorChoice::Always
        } else {
            ColorChoice::Never
        });
        if self.errors.is_empty() {
            return None;
        }

        let mut total = 0;

        // Deduplicate errors based on the location attached to the error
        let mut error_map = BTreeMap::new();
        for mut error in self.errors.drain(..) {
            if !error.notes.is_empty() {
                // Sort everything except the first element
                let first = error.notes.remove(0);
                error.notes.sort();
                error.notes.insert(0, first);
            }

            error_map
                .entry(error.notes)
                .or_insert_with(Vec::new)
                .push(error.kind);
        }

        let table = GlobalPositionTable::get();
        for (all_notes, errors) in error_map {
            let mut labels = vec![];
            let mut notes = vec![];
            for (idx, info) in all_notes.iter().enumerate() {
                let info = &self.infos[info.0];
                if let Some(p) = info.pos.into_option() {
                    let pos = table.get_pos(p.0);
                    let l = Label::new(
                        if idx == 0 {
                            LabelStyle::Primary
                        } else {
                            LabelStyle::Secondary
                        },
                        pos.file.get(),
                        pos.start..pos.end,
                    );
                    labels.push(l.with_message(info.message.clone()));
                } else {
                    notes.push(info.message.clone());
                }
            }

            let msg = if errors.len() > 1 {
                notes.extend(errors.iter().map(|e| e.to_string()));
                "Multiple errors encountered".to_string()
            } else {
                errors[0].to_string()
            };

            total += 1;
            term::emit(
                &mut writer.lock(),
                &term::Config::default(),
                table.files(),
                &Diagnostic::error()
                    .with_message(msg)
                    .with_labels(labels)
                    .with_notes(notes),
            )
            .unwrap();
        }

        Some(total)
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        if let Some(errs) = self.report_all() {
            if std::thread::panicking() {
                eprintln!("{errs} errors encountered before panic");
            }
        }
    }
}
