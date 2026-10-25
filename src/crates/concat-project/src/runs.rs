// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! A title's runs: stretches of its words with a look of their own.
//!
//! A title is one style ([`TextStyle`]) and, over it, a list of
//! [`TextRun`]s - the third word in yellow, the name in bold. A run is a
//! range of characters and the fields it overrides; everything it leaves
//! out is the title's, so changing the title's colour still changes every
//! word that was not given one of its own.
//!
//! The list is kept in one shape whatever wrote it - sorted, apart, none
//! empty, none that overrides nothing, neighbours that look the same joined
//! into one - by [`TextStyle::tidy_runs`], which the document reader and
//! every command run. Two titles that look the same then compare the same,
//! and the painter's cache does not paint one twice.
//!
//! The words change under the runs: a run is pinned to characters, and
//! when characters are typed or deleted the runs move with the words they
//! cover. The rules are the ones every word processor has, so nobody has
//! to learn them: typing inside a coloured word carries on in its colour,
//! typing just after it does too, typing just before it does not, and
//! typing over a selection takes the look of the first character it
//! replaces. See [`TextStyle::edit_runs`].

use crate::model::{RunStyle, TextCase, TextRun, TextStyle};

/// One change to a title's words, in characters: `removed` characters at
/// `at` gave way to `inserted` new ones. A keystroke is one with nothing
/// removed, a backspace one with nothing inserted, a paste over a
/// selection both.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextEdit {
    /// Where the change starts.
    pub at: usize,
    /// How many characters went.
    pub removed: usize,
    /// How many came in their place.
    pub inserted: usize,
}

impl TextEdit {
    /// The one change that turns `old` into `new`, or `None` when they are
    /// the same: whatever the two share at the front and at the back is
    /// untouched, and the middle is what changed.
    ///
    /// That alone cannot always tell where a character was typed - an "l"
    /// typed after "hel" in "hello" reads the same as one typed after
    /// "hell" - and the difference matters when a run ends between the
    /// two. `caret`, where the text cursor is after the edit (in
    /// characters of `new`), settles it: the change is slid back to end at
    /// the caret whenever the words allow that reading.
    pub fn between(old: &str, new: &str, caret: Option<usize>) -> Option<TextEdit> {
        let old: Vec<char> = old.chars().collect();
        let new: Vec<char> = new.chars().collect();
        if old == new {
            return None;
        }
        let front = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - front;
        let back = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        let removed = old.len() - front - back;
        let inserted = new.len() - front - back;
        let mut edit = TextEdit {
            at: front,
            removed,
            inserted,
        };
        if let Some(caret) = caret {
            let end = front + inserted;
            if caret < end && end - caret <= front {
                let at = front - (end - caret);
                // The front before `at` is shared by construction; what
                // follows the change must still be the same in both.
                if old[at + removed..] == new[at + inserted..] {
                    edit.at = at;
                }
            }
        }
        Some(edit)
    }
}

impl RunStyle {
    /// Every number in range - a weight on the CSS scale, a size the
    /// painter can draw - and an override that cannot mean anything, a
    /// number that is not one or a colour that is empty, dropped back to
    /// the title's.
    pub fn tidy(mut self) -> RunStyle {
        self.font_weight = self
            .font_weight
            .filter(|weight| weight.is_finite())
            .map(|weight| weight.clamp(100.0, 900.0));
        self.font_scale = self
            .font_scale
            .filter(|scale| scale.is_finite())
            .map(|scale| scale.clamp(0.1, 10.0));
        self.color = self.color.filter(|colour| !colour.trim().is_empty());
        self.stroke_color = self.stroke_color.filter(|colour| !colour.trim().is_empty());
        self
    }
}

impl TextStyle {
    /// How many characters the words are: the end of the last place a run
    /// can cover.
    pub fn char_count(&self) -> usize {
        self.content.chars().count()
    }

    /// Lays `patch` over the characters `start..end`: each field it has is
    /// what those characters wear from now on, whatever runs said before,
    /// and each it leaves out stays as it was there. The ends are clamped
    /// to the words and may come in either order; an empty range or an
    /// empty patch changes nothing.
    pub fn style_range(&mut self, start: usize, end: usize, patch: &RunStyle) {
        if patch.is_empty() {
            return;
        }
        // Later runs win where they overlap earlier ones (see
        // `tidy_runs`), so a patch laid over is a patch pushed on.
        self.runs.push(TextRun {
            start,
            end,
            style: patch.clone(),
        });
        self.tidy_runs();
    }

    /// Gives the characters `start..end` back to the title's own look:
    /// every run is cut out of the range, and the parts of it outside the
    /// range keep their look.
    pub fn clear_range(&mut self, start: usize, end: usize) {
        let (start, end) = (start.min(end), start.max(end));
        let mut kept = Vec::with_capacity(self.runs.len() + 1);
        for run in std::mem::take(&mut self.runs) {
            if run.end <= start || run.start >= end {
                kept.push(run);
                continue;
            }
            if run.start < start {
                kept.push(TextRun {
                    start: run.start,
                    end: start,
                    style: run.style.clone(),
                });
            }
            if run.end > end {
                kept.push(TextRun {
                    start: end,
                    end: run.end,
                    style: run.style,
                });
            }
        }
        self.runs = kept;
        self.tidy_runs();
    }

    /// The overrides the character at `at` wears; nothing for one in the
    /// title's own look, or past the end.
    pub fn run_style_at(&self, at: usize) -> RunStyle {
        self.runs
            .iter()
            .find(|run| run.start <= at && at < run.end)
            .map(|run| run.style.clone())
            .unwrap_or_default()
    }

    /// New words for the title, the runs carried along with the words they
    /// were on. `caret` is where the text cursor stands after the edit, in
    /// characters, when the edit came from typing; see [`TextEdit::between`].
    pub fn set_content(&mut self, content: String, caret: Option<usize>) {
        let edit = TextEdit::between(&self.content, &content, caret);
        self.content = content;
        if let Some(edit) = edit {
            self.edit_runs(edit);
        }
    }

    /// Moves the runs for one change of the words (the content has already
    /// changed): a run after the change moves by however much the words
    /// grew or shrank, one the change cuts into loses the characters that
    /// went, and the new characters join the run whose look they take -
    ///
    /// - typed with nothing selected, the look of the character before the
    ///   caret, or at the very start the look of the first character;
    /// - typed over a selection, the look of the first character replaced.
    ///
    /// A run whose every character was deleted is gone.
    pub fn edit_runs(&mut self, edit: TextEdit) {
        if self.runs.is_empty() {
            return;
        }
        let TextEdit {
            at,
            removed,
            inserted,
        } = edit;
        let owner = if removed > 0 {
            self.runs
                .iter()
                .position(|run| run.start <= at && at < run.end)
        } else if at > 0 {
            self.runs
                .iter()
                .position(|run| run.start < at && at <= run.end)
        } else {
            self.runs
                .iter()
                .position(|run| run.start == 0 && run.end > 0)
        };
        let cut_end = at + removed;
        // Where a character boundary lands once the removed ones are gone.
        let map = |x: usize| {
            if x <= at {
                x
            } else if x <= cut_end {
                at
            } else {
                x - removed
            }
        };
        for (index, run) in self.runs.iter_mut().enumerate() {
            let (start, end) = (map(run.start), map(run.end));
            if Some(index) == owner {
                // It starts at or before the change and reaches it, so the
                // new characters land inside it.
                run.start = start;
                run.end = end + inserted;
            } else if start >= at && end > at {
                run.start = start + inserted;
                run.end = end + inserted;
            } else {
                // Wholly before the change, or emptied by it.
                run.start = start;
                run.end = end;
            }
        }
        self.tidy_runs();
    }

    /// The runs in their one shape: clamped to the words, each override
    /// tidied, the empty ones and those that override nothing dropped,
    /// overlaps resolved - where two runs cover a character, the later
    /// one's fields win - and neighbours that look the same joined. Cheap
    /// for a title without runs, which is nearly every one.
    pub fn tidy_runs(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let count = self.char_count();
        let runs: Vec<TextRun> = std::mem::take(&mut self.runs)
            .into_iter()
            .filter_map(|run| {
                let style = run.style.tidy();
                let start = run.start.min(run.end).min(count);
                let end = run.start.max(run.end).min(count);
                (start < end && !style.is_empty()).then_some(TextRun { start, end, style })
            })
            .collect();
        let mut cuts: Vec<usize> = runs.iter().flat_map(|run| [run.start, run.end]).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut out: Vec<TextRun> = Vec::with_capacity(runs.len());
        for pair in cuts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let style = runs
                .iter()
                .filter(|run| run.start <= start && end <= run.end)
                .fold(RunStyle::default(), |held, run| held.and(&run.style));
            if style.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.end == start && last.style == style => last.end = end,
                _ => out.push(TextRun { start, end, style }),
            }
        }
        self.runs = out;
    }

    /// The words as they are painted, and the runs on them: the content in
    /// the style's case, with each run moved to the characters its own
    /// became. Casing can change how many characters there are - "ß" is
    /// "SS" in capitals - so a run cannot simply keep its numbers.
    ///
    /// Sentence case lowercases the words and capitalises the first letter
    /// of each sentence: the first one, and those after `.`, `!`, `?` and
    /// their full-width forms. Punctuation alone, with no knowledge of a
    /// language's abbreviations.
    pub fn display(&self) -> (String, Vec<TextRun>) {
        let content = &self.content;
        let (text, widths): (String, Vec<usize>) = match self.text_case {
            TextCase::Original => return (content.clone(), self.runs.clone()),
            TextCase::Upper => (
                content.to_uppercase(),
                content
                    .chars()
                    .map(|ch| ch.to_uppercase().count())
                    .collect(),
            ),
            TextCase::Lower => (
                content.to_lowercase(),
                content
                    .chars()
                    .map(|ch| ch.to_lowercase().count())
                    .collect(),
            ),
            TextCase::Sentence => {
                // The whole string is lowercased at once, as a final
                // sigma needs, and walked with each source character's
                // share of it.
                let lowered: Vec<char> = content.to_lowercase().chars().collect();
                let mut text = String::with_capacity(content.len());
                let mut widths = Vec::with_capacity(lowered.len());
                let mut initial = true;
                let mut next = 0;
                for ch in content.chars() {
                    let share = ch.to_lowercase().count();
                    let pieces = lowered.get(next..next + share).unwrap_or(&[]);
                    next += share;
                    let mut width = 0;
                    for &piece in pieces {
                        if initial && piece.is_alphabetic() {
                            for upper in piece.to_uppercase() {
                                text.push(upper);
                                width += 1;
                            }
                            initial = false;
                        } else {
                            text.push(piece);
                            width += 1;
                        }
                        if matches!(piece, '.' | '!' | '?' | '。' | '！' | '？') {
                            initial = true;
                        }
                    }
                    widths.push(width);
                }
                (text, widths)
            }
        };
        if self.runs.is_empty() {
            return (text, Vec::new());
        }
        let mut edges = Vec::with_capacity(widths.len() + 1);
        edges.push(0);
        for width in widths {
            edges.push(edges.last().copied().unwrap_or(0) + width);
        }
        let last = edges.last().copied().unwrap_or(0);
        let edge = |at: usize| edges.get(at).copied().unwrap_or(last);
        let runs = self
            .runs
            .iter()
            .map(|run| TextRun {
                start: edge(run.start),
                end: edge(run.end),
                style: run.style.clone(),
            })
            .filter(|run| run.start < run.end)
            .collect();
        (text, runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> RunStyle {
        RunStyle {
            color: Some("#ff0000".into()),
            ..RunStyle::default()
        }
    }

    fn bold() -> RunStyle {
        RunStyle {
            font_weight: Some(700.0),
            ..RunStyle::default()
        }
    }

    fn title(content: &str) -> TextStyle {
        TextStyle {
            content: content.into(),
            ..TextStyle::default()
        }
    }

    fn spans(style: &TextStyle) -> Vec<(usize, usize)> {
        style.runs.iter().map(|run| (run.start, run.end)).collect()
    }

    /// "hello red world" with "red" red.
    fn with_red() -> TextStyle {
        let mut style = title("hello red world");
        style.style_range(6, 9, &red());
        style
    }

    #[test]
    fn a_range_is_styled_clamped_and_in_either_order() {
        let mut style = title("hello");
        style.style_range(10, 3, &red());
        assert_eq!(spans(&style), [(3, 5)]);
        style.style_range(2, 2, &bold());
        assert_eq!(spans(&style), [(3, 5)], "an empty range changes nothing");
        style.style_range(0, 2, &RunStyle::default());
        assert_eq!(spans(&style), [(3, 5)], "an empty patch changes nothing");
    }

    #[test]
    fn a_patch_over_part_of_a_run_splits_it_and_keeps_what_it_leaves_out() {
        let mut style = title("one two three");
        style.style_range(0, 13, &red());
        style.style_range(4, 7, &bold());
        assert_eq!(spans(&style), [(0, 4), (4, 7), (7, 13)]);
        assert_eq!(style.runs[1].style, red().and(&bold()));
        assert_eq!(style.run_style_at(5).color.as_deref(), Some("#ff0000"));
        assert_eq!(style.run_style_at(5).font_weight, Some(700.0));
        assert!(style.run_style_at(13).is_empty(), "past the end");
    }

    #[test]
    fn neighbours_that_look_the_same_are_joined() {
        let mut style = title("abcdef");
        style.style_range(0, 2, &red());
        style.style_range(2, 4, &red());
        assert_eq!(spans(&style), [(0, 4)]);
        style.style_range(5, 6, &red());
        assert_eq!(spans(&style), [(0, 4), (5, 6)], "apart stays apart");
    }

    #[test]
    fn clearing_cuts_the_range_out_of_every_run() {
        let mut style = title("one two three");
        style.style_range(0, 13, &red());
        style.clear_range(4, 7);
        assert_eq!(spans(&style), [(0, 4), (7, 13)]);
        style.clear_range(0, 100);
        assert!(style.runs.is_empty());
    }

    #[test]
    fn overlapping_runs_from_a_file_resolve_with_the_later_winning() {
        let mut style = title("abcdef");
        style.runs = vec![
            TextRun {
                start: 0,
                end: 4,
                style: red(),
            },
            TextRun {
                start: 2,
                end: 6,
                style: RunStyle {
                    color: Some("#00ff00".into()),
                    ..RunStyle::default()
                },
            },
            TextRun {
                start: 4,
                end: 3,
                style: RunStyle {
                    font_scale: Some(f64::NAN),
                    ..RunStyle::default()
                },
            },
        ];
        style.tidy_runs();
        assert_eq!(spans(&style), [(0, 2), (2, 6)]);
        assert_eq!(style.runs[1].style.color.as_deref(), Some("#00ff00"));
    }

    #[test]
    fn numbers_are_held_in_range() {
        let mut style = title("abc");
        style.style_range(
            0,
            3,
            &RunStyle {
                font_weight: Some(5000.0),
                font_scale: Some(0.0),
                ..RunStyle::default()
            },
        );
        assert_eq!(style.runs[0].style.font_weight, Some(900.0));
        assert_eq!(style.runs[0].style.font_scale, Some(0.1));
    }

    #[test]
    fn the_edit_between_two_texts_is_their_middle() {
        assert_eq!(TextEdit::between("same", "same", None), None);
        assert_eq!(
            TextEdit::between("hello world", "hello big world", None),
            Some(TextEdit {
                at: 6,
                removed: 0,
                inserted: 4
            })
        );
        assert_eq!(
            TextEdit::between("hello red world", "hello blue world", None),
            Some(TextEdit {
                at: 6,
                removed: 3,
                inserted: 4
            })
        );
        // Characters, not bytes.
        assert_eq!(
            TextEdit::between("çà va", "çàx va", None),
            Some(TextEdit {
                at: 2,
                removed: 0,
                inserted: 1
            })
        );
    }

    #[test]
    fn the_caret_settles_where_a_repeated_letter_was_typed() {
        // "hel|lo" + "l": the texts alone read it as typed after "hell".
        assert_eq!(TextEdit::between("hello", "helllo", None).unwrap().at, 4);
        assert_eq!(TextEdit::between("hello", "helllo", Some(4)).unwrap().at, 3);
        // A caret the words cannot agree with is not followed.
        assert_eq!(TextEdit::between("hello", "helxlo", Some(1)).unwrap().at, 3);
        // A backspace on the first "l" of two.
        assert_eq!(TextEdit::between("hello", "helo", Some(2)).unwrap().at, 2);
    }

    #[test]
    fn typing_inside_a_coloured_word_keeps_its_colour() {
        let mut style = with_red();
        style.set_content("hello rexd world".into(), Some(9));
        assert_eq!(spans(&style), [(6, 10)]);
    }

    #[test]
    fn typing_just_after_a_coloured_word_carries_its_colour_on() {
        let mut style = with_red();
        style.set_content("hello reds world".into(), Some(10));
        assert_eq!(spans(&style), [(6, 10)]);
    }

    #[test]
    fn typing_just_before_a_coloured_word_is_in_the_plain_look() {
        let mut style = with_red();
        style.set_content("hello xred world".into(), Some(7));
        assert_eq!(spans(&style), [(7, 10)]);
    }

    #[test]
    fn typing_at_the_very_start_takes_the_first_characters_look() {
        let mut style = title("red rest");
        style.style_range(0, 3, &red());
        style.set_content("Xred rest".into(), Some(1));
        assert_eq!(spans(&style), [(0, 4)]);
    }

    #[test]
    fn typing_over_a_selection_takes_the_look_of_its_first_character() {
        let mut style = with_red();
        style.set_content("hello blue world".into(), Some(10));
        assert_eq!(spans(&style), [(6, 10)], "over the red word, red");
        let mut style = with_red();
        // "o r" replaced: its first character was plain.
        style.set_content("hellXed world".into(), Some(5));
        assert_eq!(spans(&style), [(5, 7)]);
    }

    #[test]
    fn a_run_moves_with_the_words_before_it() {
        let mut style = with_red();
        style.set_content("oh hello red world".into(), Some(3));
        assert_eq!(spans(&style), [(9, 12)]);
        style.set_content("hello red world".into(), Some(0));
        assert_eq!(spans(&style), [(6, 9)]);
    }

    #[test]
    fn a_deletion_shortens_a_run_and_a_whole_one_goes() {
        let mut style = with_red();
        // "o r" deleted: the run keeps "ed".
        style.set_content("helled world".into(), Some(4));
        assert_eq!(spans(&style), [(4, 6)]);
        let mut style = with_red();
        style.set_content("hello  world".into(), Some(6));
        assert!(style.runs.is_empty());
        let mut style = with_red();
        style.set_content(String::new(), Some(0));
        assert!(style.runs.is_empty());
    }

    #[test]
    fn edits_after_a_run_leave_it_alone() {
        let mut style = with_red();
        style.set_content("hello red world!".into(), Some(16));
        assert_eq!(spans(&style), [(6, 9)]);
    }

    #[test]
    fn case_moves_runs_to_the_characters_they_become() {
        let mut style = title("straße rot");
        style.style_range(7, 10, &red());
        style.text_case = TextCase::Upper;
        let (text, runs) = style.display();
        assert_eq!(text, "STRASSE ROT");
        assert_eq!((runs[0].start, runs[0].end), (8, 11));
        style.text_case = TextCase::Sentence;
        let (text, runs) = style.display();
        assert_eq!(text, "Straße rot");
        assert_eq!((runs[0].start, runs[0].end), (7, 10));
        style.text_case = TextCase::Original;
        assert_eq!(style.display().1, style.runs);
    }

    #[test]
    fn sentence_case_reads_the_same_as_it_always_did() {
        let style = TextStyle {
            content: "«È UNA PROVA!» POI, UN’ALTRA. già qui? sì! ΟΔΟΣ.".into(),
            text_case: TextCase::Sentence,
            ..TextStyle::default()
        };
        assert_eq!(
            style.display_content(),
            "«È una prova!» Poi, un’altra. Già qui? Sì! Οδος."
        );
    }

    #[test]
    fn runs_round_trip_and_a_title_without_them_writes_none() {
        let plain = serde_json::to_value(title("hi")).unwrap();
        assert!(plain.get("runs").is_none());
        let style = with_red();
        let saved = serde_json::to_value(&style).unwrap();
        assert_eq!(
            saved["runs"],
            serde_json::json!([{ "start": 6, "end": 9, "color": "#ff0000" }])
        );
        let back: TextStyle = serde_json::from_value(saved).unwrap();
        assert_eq!(back, style);
        // A run that does not parse is dropped, not the title.
        let odd: TextStyle = serde_json::from_value(serde_json::json!({
            "content": "abc",
            "runs": [{ "start": "x" }, { "start": 0, "end": 1, "italic": true }],
        }))
        .unwrap();
        assert_eq!(odd.runs.len(), 1);
    }
}
