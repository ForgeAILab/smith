//! Pure questionnaire presentation and keyboard state.
//!
//! Agent Runtime owns the versioned interaction protocol and turn resumption.
//! This module owns only Smith's projection of one bounded request: one visible
//! question at a time, staged answers, explicit navigation/actions, and no I/O.
//! The host adapter maps runtime request/response types at the crate boundary;
//! the reducer never calls approval or grants authority.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use agent_runtime_core::clock::Deadline;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::composer::Composer;

/// Maximum questions accepted by Smith's temporary questionnaire overlay.
pub const MAX_QUESTIONS: usize = 3;
/// Maximum labelled choices rendered for one question.
pub const MAX_CHOICES: usize = 8;
/// Maximum free-form answer length, counted in Unicode scalar values.
///
/// This intentionally matches Agent Runtime's public interaction contract so
/// every valid broker request remains answerable without a hidden client-side
/// narrowing.
pub const MAX_FREE_FORM_CHARS: usize = 8_192;

/// One labelled choice projected into the terminal.
#[derive(Clone, PartialEq, Eq)]
pub struct QuestionnaireChoice {
    /// Stable protocol identity returned in an answer.
    pub id: String,
    /// Short user-facing label.
    pub label: String,
    /// Optional bounded consequence or tradeoff.
    pub description: Option<String>,
}

impl fmt::Debug for QuestionnaireChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionnaireChoice")
            .field("has_description", &self.description.is_some())
            .finish_non_exhaustive()
    }
}

impl QuestionnaireChoice {
    /// Creates a choice with no secondary description.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
        }
    }

    /// Adds a secondary consequence/tradeoff line.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// One question in a bounded questionnaire.
#[derive(Clone, PartialEq, Eq)]
pub struct QuestionnaireQuestion {
    /// Stable protocol identity returned with the answer.
    pub id: String,
    /// Short heading shown before the prompt.
    pub header: String,
    /// The question text.
    pub prompt: String,
    /// Mutually exclusive labelled choices.
    pub choices: Vec<QuestionnaireChoice>,
    /// Whether a free-form answer is accepted instead of a choice.
    pub allows_free_form: bool,
    /// Whether Smith must mask and redact the free-form value.
    pub sensitive: bool,
}

impl QuestionnaireQuestion {
    /// Creates a choice-only question.
    pub fn new(
        id: impl Into<String>,
        header: impl Into<String>,
        prompt: impl Into<String>,
        choices: Vec<QuestionnaireChoice>,
    ) -> Self {
        Self {
            id: id.into(),
            header: header.into(),
            prompt: prompt.into(),
            choices,
            allows_free_form: false,
            sensitive: false,
        }
    }

    /// Allows an alternative free-form answer.
    #[must_use]
    pub fn with_free_form(mut self, sensitive: bool) -> Self {
        self.allows_free_form = true;
        self.sensitive = sensitive;
        self
    }

    /// Applies the request's sensitivity even when the question is
    /// choice-only.
    #[must_use]
    pub fn with_sensitivity(mut self, sensitive: bool) -> Self {
        self.sensitive = sensitive;
        self
    }
}

impl fmt::Debug for QuestionnaireQuestion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionnaireQuestion")
            .field("choice_count", &self.choices.len())
            .field("allows_free_form", &self.allows_free_form)
            .field("sensitive", &self.sensitive)
            .finish_non_exhaustive()
    }
}

/// Protocol-neutral data shown by the questionnaire overlay.
#[derive(Clone, PartialEq, Eq)]
pub struct QuestionnaireForm {
    /// Stable runtime request identity.
    pub request_id: String,
    /// One to three questions in wire order.
    pub questions: Vec<QuestionnaireQuestion>,
    /// Runtime-enforced absolute deadline.
    pub deadline: Deadline,
    /// Whether this request was restored from a protected checkpoint.
    pub restored: bool,
}

impl fmt::Debug for QuestionnaireForm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionnaireForm")
            .field("question_count", &self.questions.len())
            .field(
                "sensitive_questions",
                &self
                    .questions
                    .iter()
                    .filter(|question| question.sensitive)
                    .count(),
            )
            .field("restored", &self.restored)
            .finish_non_exhaustive()
    }
}

impl QuestionnaireForm {
    /// Validates and creates a bounded questionnaire.
    pub fn new(
        request_id: impl Into<String>,
        questions: Vec<QuestionnaireQuestion>,
        deadline: Deadline,
    ) -> Result<Self, QuestionnaireValidationError> {
        let form = Self {
            request_id: request_id.into(),
            questions,
            deadline,
            restored: false,
        };
        form.validate()?;
        Ok(form)
    }

    /// Labels this request as restored protected state.
    #[must_use]
    pub fn restored(mut self, restored: bool) -> Self {
        self.restored = restored;
        self
    }

    fn validate(&self) -> Result<(), QuestionnaireValidationError> {
        if self.request_id.trim().is_empty() {
            return Err(QuestionnaireValidationError::new(
                "questionnaire request id cannot be empty",
            ));
        }
        if !(1..=MAX_QUESTIONS).contains(&self.questions.len()) {
            return Err(QuestionnaireValidationError::new(format!(
                "questionnaire must contain between 1 and {MAX_QUESTIONS} questions"
            )));
        }
        let mut question_ids = BTreeSet::new();
        for question in &self.questions {
            if question.id.trim().is_empty() || !question_ids.insert(question.id.clone()) {
                return Err(QuestionnaireValidationError::new(
                    "question ids must be non-empty and unique",
                ));
            }
            if question.header.trim().is_empty() || question.prompt.trim().is_empty() {
                return Err(QuestionnaireValidationError::new(
                    "question header and prompt cannot be empty",
                ));
            }
            if question.choices.len() > MAX_CHOICES {
                return Err(QuestionnaireValidationError::new(format!(
                    "one question cannot contain more than {MAX_CHOICES} choices"
                )));
            }
            if question.choices.is_empty() && !question.allows_free_form {
                return Err(QuestionnaireValidationError::new(
                    "a question needs a choice or a free-form answer",
                ));
            }
            let mut choice_ids = BTreeSet::new();
            for choice in &question.choices {
                if choice.id.trim().is_empty()
                    || choice.label.trim().is_empty()
                    || !choice_ids.insert(choice.id.clone())
                {
                    return Err(QuestionnaireValidationError::new(
                        "choice ids must be unique and choice labels cannot be empty",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Invalid UI projection supplied by a host adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionnaireValidationError {
    message: String,
}

impl QuestionnaireValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for QuestionnaireValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for QuestionnaireValidationError {}

/// The answer selected for one question.
#[derive(Clone, PartialEq, Eq)]
pub enum QuestionnaireAnswerValue {
    /// Stable labelled-choice identity.
    Choice(String),
    /// User-authored free-form text.
    FreeForm(String),
}

/// One typed answer returned to the interaction adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct QuestionnaireAnswer {
    /// Stable question identity.
    pub question_id: String,
    /// Selected choice or free-form value.
    pub value: QuestionnaireAnswerValue,
    /// Whether logs/debugging must redact the value.
    pub sensitive: bool,
}

impl fmt::Debug for QuestionnaireAnswer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionnaireAnswer")
            .field(
                "answer_kind",
                &match &self.value {
                    QuestionnaireAnswerValue::Choice(_) => "choice",
                    QuestionnaireAnswerValue::FreeForm(_) => "free_form",
                },
            )
            .field("sensitive", &self.sensitive)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for QuestionnaireAnswerValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Choice(_) => formatter.write_str("Choice([protected])"),
            Self::FreeForm(_) => formatter.write_str("FreeForm([protected])"),
        }
    }
}

/// Explicit terminal result from the temporary questionnaire surface.
#[derive(Clone, PartialEq, Eq)]
pub enum QuestionnaireResolution {
    /// Every staged answer, in question order.
    Submitted(Vec<QuestionnaireAnswer>),
    /// The user explicitly declined to answer.
    Declined,
    /// The active turn or user cancelled the interaction.
    Cancelled,
    /// The runtime deadline elapsed without an answer.
    TimedOut,
}

impl fmt::Debug for QuestionnaireResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Submitted(answers) => formatter
                .debug_struct("QuestionnaireResolution::Submitted")
                .field("answer_count", &answers.len())
                .finish(),
            Self::Declined => formatter.write_str("QuestionnaireResolution::Declined"),
            Self::Cancelled => formatter.write_str("QuestionnaireResolution::Cancelled"),
            Self::TimedOut => formatter.write_str("QuestionnaireResolution::TimedOut"),
        }
    }
}

/// Pure reducer state for one questionnaire.
///
/// Mirrors Claude Code's question picker: one question per tab, a list whose
/// last row (when allowed) is a free-form "Type something" field, Enter or a
/// number picks the highlighted row and moves on, and a multi-question form
/// ends on a review tab that submits or declines. A single question submits
/// as soon as it is answered.
pub struct QuestionnaireState {
    form: QuestionnaireForm,
    /// Visible tab; `questions.len()` is the review tab.
    current: usize,
    /// Highlighted row per tab, review tab last.
    cursors: Vec<usize>,
    staged: BTreeMap<String, QuestionnaireAnswerValue>,
    drafts: BTreeMap<String, Composer>,
    error: Option<String>,
}

impl fmt::Debug for QuestionnaireState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuestionnaireState")
            .field("request_id", &self.form.request_id)
            .field("current", &self.current)
            .field("staged_answers", &self.staged.len())
            .field("restored", &self.form.restored)
            .finish_non_exhaustive()
    }
}

/// Rows on the review tab, in order.
pub const REVIEW_ROWS: [&str; 2] = ["Submit answers", "Decline to answer"];

impl QuestionnaireState {
    /// Starts on the first question with its first row highlighted.
    pub fn new(form: QuestionnaireForm) -> Self {
        let tabs = form.questions.len() + 1;
        Self {
            form,
            current: 0,
            cursors: vec![0; tabs],
            staged: BTreeMap::new(),
            drafts: BTreeMap::new(),
            error: None,
        }
    }

    /// The immutable projected request.
    pub fn form(&self) -> &QuestionnaireForm {
        &self.form
    }

    /// Whether the form has a review tab (more than one question).
    pub fn has_review(&self) -> bool {
        self.form.questions.len() > 1
    }

    /// Whether the review tab is visible.
    pub fn is_review(&self) -> bool {
        self.current == self.form.questions.len()
    }

    /// The visible question, or the last one while reviewing.
    pub fn question(&self) -> &QuestionnaireQuestion {
        &self.form.questions[self.current.min(self.form.questions.len() - 1)]
    }

    /// Zero-based visible tab; equals the question count on the review tab.
    pub fn current_index(&self) -> usize {
        self.current
    }

    /// Highlighted row on the visible tab.
    pub fn cursor(&self) -> usize {
        self.cursors[self.current]
    }

    /// Whether the highlighted row is the free-form field.
    pub fn on_free_form_row(&self) -> bool {
        !self.is_review()
            && self.question().allows_free_form
            && self.cursor() == self.question().choices.len()
    }

    /// Local validation error shown inside the overlay.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether question `index` has a staged answer.
    pub fn is_answered(&self, index: usize) -> bool {
        self.form
            .questions
            .get(index)
            .is_some_and(|question| self.staged.contains_key(&question.id))
    }

    /// The current free-form draft, masked when sensitive.
    pub fn displayed_draft(&self) -> String {
        let Some(draft) = self.drafts.get(&self.question().id) else {
            return String::new();
        };
        if self.question().sensitive {
            "*".repeat(draft.len())
        } else {
            draft.text().to_owned()
        }
    }

    /// The staged choice ID for the visible question, if any.
    pub fn staged_choice(&self) -> Option<&str> {
        match self.staged.get(&self.question().id) {
            Some(QuestionnaireAnswerValue::Choice(choice)) => Some(choice),
            _ => None,
        }
    }

    /// The staged answer for question `index` as display text; sensitive
    /// free-form answers are masked.
    pub fn answer_text(&self, index: usize) -> Option<String> {
        let question = self.form.questions.get(index)?;
        match self.staged.get(&question.id)? {
            QuestionnaireAnswerValue::Choice(id) => question
                .choices
                .iter()
                .find(|choice| &choice.id == id)
                .map(|choice| choice.label.clone()),
            QuestionnaireAnswerValue::FreeForm(text) if question.sensitive => {
                Some("*".repeat(text.chars().count()))
            }
            QuestionnaireAnswerValue::FreeForm(text) => Some(text.clone()),
        }
    }

    fn row_count(&self) -> usize {
        if self.is_review() {
            REVIEW_ROWS.len()
        } else {
            self.question().choices.len() + usize::from(self.question().allows_free_form)
        }
    }

    /// Handles one key without performing I/O.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<QuestionnaireResolution> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        let editing = self.on_free_form_row();
        match key.code {
            KeyCode::Esc => return Some(QuestionnaireResolution::Cancelled),
            KeyCode::Up => self.move_cursor(false),
            KeyCode::Down => self.move_cursor(true),
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => self.move_cursor(false),
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => self.move_cursor(true),
            KeyCode::Tab => self.switch_tab(true),
            KeyCode::BackTab => self.switch_tab(false),
            KeyCode::Right if !editing => self.switch_tab(true),
            KeyCode::Left if !editing => self.switch_tab(false),
            KeyCode::Enter => return self.activate(),
            _ if editing => self.edit_draft(key, plain),
            KeyCode::Char(number @ '1'..='9') if plain => {
                let index = usize::from(number as u8 - b'1');
                if index < self.row_count() {
                    self.cursors[self.current] = index;
                    self.error = None;
                    if !self.on_free_form_row() {
                        return self.activate();
                    }
                }
            }
            // Typing on a choice row starts the free-form answer.
            KeyCode::Char(character)
                if plain && !character.is_whitespace() && self.question_allows_typing() =>
            {
                self.cursors[self.current] = self.question().choices.len();
                self.insert_free_form(character);
            }
            _ => {}
        }
        None
    }

    fn question_allows_typing(&self) -> bool {
        !self.is_review() && self.question().allows_free_form
    }

    fn move_cursor(&mut self, down: bool) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let cursor = &mut self.cursors[self.current];
        *cursor = if down {
            (*cursor + 1) % count
        } else {
            cursor.checked_sub(1).unwrap_or(count - 1)
        };
        self.error = None;
    }

    fn switch_tab(&mut self, forward: bool) {
        let last = if self.has_review() {
            self.form.questions.len()
        } else {
            self.form.questions.len() - 1
        };
        self.current = if forward {
            (self.current + 1).min(last)
        } else {
            self.current.saturating_sub(1)
        };
        self.error = None;
    }

    fn edit_draft(&mut self, key: KeyEvent, plain: bool) {
        match key.code {
            KeyCode::Backspace => {
                self.draft_mut().backspace();
                self.stage_draft();
            }
            KeyCode::Delete => {
                self.draft_mut().delete();
                self.stage_draft();
            }
            KeyCode::Left => self.draft_mut().move_left(),
            KeyCode::Right => self.draft_mut().move_right(),
            KeyCode::Home => self.draft_mut().move_home(),
            KeyCode::End => self.draft_mut().move_end(),
            KeyCode::Char(character) if plain => self.insert_free_form(character),
            _ => {}
        }
    }

    fn activate(&mut self) -> Option<QuestionnaireResolution> {
        if self.is_review() {
            return if self.cursor() == 0 {
                self.submit()
            } else {
                Some(QuestionnaireResolution::Declined)
            };
        }
        if self.on_free_form_row() {
            self.stage_draft();
            if !self.is_answered(self.current) {
                self.error = Some("type an answer first".to_owned());
                return None;
            }
        } else {
            let question_id = self.question().id.clone();
            let choice = self.question().choices.get(self.cursor())?;
            let value = QuestionnaireAnswerValue::Choice(choice.id.clone());
            self.staged.insert(question_id, value);
        }
        self.error = None;
        self.advance()
    }

    /// Moves past an answered question: to the next unanswered one, then to
    /// the review tab; a lone question submits at once.
    fn advance(&mut self) -> Option<QuestionnaireResolution> {
        if !self.has_review() {
            return self.submit();
        }
        let count = self.form.questions.len();
        self.current = (self.current + 1..count)
            .chain(0..self.current)
            .find(|index| !self.is_answered(*index))
            .unwrap_or(count);
        None
    }

    fn insert_free_form(&mut self, character: char) {
        if character.is_control() || self.draft_mut().len() >= MAX_FREE_FORM_CHARS {
            return;
        }
        self.draft_mut().insert(character);
        self.stage_draft();
    }

    /// Inserts pasted text into the free-form draft, flattened to one line.
    ///
    /// Digits and spaces in a paste are draft characters, never choice
    /// shortcuts — only deliberate key presses select answers.
    pub fn paste(&mut self, text: &str) {
        if !self.question_allows_typing() {
            return;
        }
        self.cursors[self.current] = self.question().choices.len();
        let mut pending_space = false;
        for character in text.chars() {
            if character.is_whitespace() {
                pending_space = true;
                continue;
            }
            if character.is_control() {
                continue;
            }
            if std::mem::take(&mut pending_space) && !self.draft_mut().is_empty() {
                self.insert_free_form(' ');
            }
            self.insert_free_form(character);
        }
    }

    fn draft_mut(&mut self) -> &mut Composer {
        let question = self.question().id.clone();
        self.drafts.entry(question).or_default()
    }

    /// Mirrors the draft into the staged answer. Clearing the draft only
    /// withdraws a free-form answer; a picked choice survives it.
    fn stage_draft(&mut self) {
        let question_id = self.question().id.clone();
        let value = self
            .drafts
            .get(&question_id)
            .map(Composer::text)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !value.is_empty() {
            self.staged
                .insert(question_id, QuestionnaireAnswerValue::FreeForm(value));
        } else if matches!(
            self.staged.get(&question_id),
            Some(QuestionnaireAnswerValue::FreeForm(_))
        ) {
            self.staged.remove(&question_id);
        }
        self.error = None;
    }

    fn submit(&mut self) -> Option<QuestionnaireResolution> {
        if let Some(index) = (0..self.form.questions.len()).find(|index| !self.is_answered(*index))
        {
            self.current = index;
            self.error = Some("answer every question before submitting".to_owned());
            return None;
        }
        let answers = self
            .form
            .questions
            .iter()
            .map(|question| QuestionnaireAnswer {
                question_id: question.id.clone(),
                value: self
                    .staged
                    .get(&question.id)
                    .expect("unanswered questions were rejected")
                    .clone(),
                sensitive: question.sensitive,
            })
            .collect();
        Some(QuestionnaireResolution::Submitted(answers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str, label: &str) -> QuestionnaireChoice {
        QuestionnaireChoice::new(id, label)
    }

    fn form() -> QuestionnaireForm {
        QuestionnaireForm::new(
            "interaction-1",
            vec![
                QuestionnaireQuestion::new(
                    "design",
                    "Design",
                    "Which direction?",
                    vec![choice("minimal", "Minimal"), choice("dense", "Dense")],
                )
                .with_free_form(false),
                QuestionnaireQuestion::new("note", "Note", "Anything else?", Vec::new())
                    .with_free_form(true),
            ],
            Deadline::never(),
        )
        .unwrap()
    }

    fn single(free_form: bool) -> QuestionnaireForm {
        let question = QuestionnaireQuestion::new(
            "q",
            "Plan",
            "How long?",
            vec![choice("short", "Short"), choice("long", "Long")],
        );
        let question = if free_form {
            question.with_free_form(false)
        } else {
            question
        };
        QuestionnaireForm::new("single", vec![question], Deadline::never()).unwrap()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn typed(state: &mut QuestionnaireState, text: &str) -> Option<QuestionnaireResolution> {
        text.chars()
            .find_map(|character| state.on_key(key(KeyCode::Char(character))))
    }

    fn submitted(resolution: Option<QuestionnaireResolution>) -> Vec<QuestionnaireAnswer> {
        match resolution {
            Some(QuestionnaireResolution::Submitted(answers)) => answers,
            other => panic!("expected a submission, got {other:?}"),
        }
    }

    #[test]
    fn a_single_question_submits_on_enter() {
        let mut state = QuestionnaireState::new(single(false));
        state.on_key(key(KeyCode::Down));
        let answers = submitted(state.on_key(key(KeyCode::Enter)));
        assert_eq!(
            answers[0].value,
            QuestionnaireAnswerValue::Choice("long".to_owned())
        );
    }

    #[test]
    fn a_number_picks_and_submits_a_single_question() {
        let mut state = QuestionnaireState::new(single(false));
        let answers = submitted(state.on_key(key(KeyCode::Char('1'))));
        assert_eq!(
            answers[0].value,
            QuestionnaireAnswerValue::Choice("short".to_owned())
        );
    }

    #[test]
    fn free_form_accepts_digits_and_spaces_and_wins_over_the_highlight() {
        let mut state = QuestionnaireState::new(single(true));
        // Typing on a choice row jumps to the free-form row.
        assert_eq!(typed(&mut state, "about 2 weeks"), None);
        assert!(state.on_free_form_row());
        assert_eq!(state.displayed_draft(), "about 2 weeks");
        let answers = submitted(state.on_key(key(KeyCode::Enter)));
        assert_eq!(
            answers[0].value,
            QuestionnaireAnswerValue::FreeForm("about 2 weeks".to_owned())
        );
    }

    #[test]
    fn enter_on_an_empty_free_form_row_explains_instead_of_submitting() {
        let mut state = QuestionnaireState::new(single(true));
        state.on_key(key(KeyCode::Char('3')));
        assert!(state.on_free_form_row());
        assert_eq!(state.on_key(key(KeyCode::Enter)), None);
        assert!(state.error().is_some());
    }

    #[test]
    fn clearing_a_draft_keeps_a_picked_choice() {
        let mut state = QuestionnaireState::new(form());
        state.on_key(key(KeyCode::Char('2')));
        // Back to the first question; the choice is staged.
        state.on_key(key(KeyCode::BackTab));
        state.on_key(key(KeyCode::Char('3')));
        state.on_key(key(KeyCode::Backspace));
        assert_eq!(state.staged_choice(), Some("dense"));
    }

    #[test]
    fn answering_moves_on_and_the_review_tab_submits_in_question_order() {
        let mut state = QuestionnaireState::new(form());
        assert_eq!(state.on_key(key(KeyCode::Char('2'))), None);
        assert_eq!(state.current_index(), 1);
        assert_eq!(typed(&mut state, "ship it"), None);
        assert_eq!(state.on_key(key(KeyCode::Enter)), None);
        assert!(state.is_review());
        assert_eq!(state.answer_text(0).as_deref(), Some("Dense"));
        let answers = submitted(state.on_key(key(KeyCode::Enter)));
        assert_eq!(
            answers
                .iter()
                .map(|answer| answer.question_id.as_str())
                .collect::<Vec<_>>(),
            vec!["design", "note"]
        );
        assert_eq!(
            answers[1].value,
            QuestionnaireAnswerValue::FreeForm("ship it".to_owned())
        );
    }

    #[test]
    fn submitting_with_a_gap_returns_to_the_unanswered_question() {
        let mut state = QuestionnaireState::new(form());
        state.on_key(key(KeyCode::Tab));
        state.on_key(key(KeyCode::Tab));
        assert!(state.is_review());
        assert_eq!(state.on_key(key(KeyCode::Enter)), None);
        assert_eq!(state.current_index(), 0);
        assert!(state.error().is_some());
    }

    #[test]
    fn decline_and_cancel_are_distinct_typed_outcomes() {
        let mut decline = QuestionnaireState::new(form());
        decline.on_key(key(KeyCode::Tab));
        decline.on_key(key(KeyCode::Tab));
        decline.on_key(key(KeyCode::Down));
        assert_eq!(
            decline.on_key(key(KeyCode::Enter)),
            Some(QuestionnaireResolution::Declined)
        );

        let mut cancel = QuestionnaireState::new(form());
        assert_eq!(
            cancel.on_key(key(KeyCode::Esc)),
            Some(QuestionnaireResolution::Cancelled)
        );
    }

    #[test]
    fn sensitive_free_form_is_masked_and_redacted_from_debug() {
        let form = QuestionnaireForm::new(
            "interaction-sensitive",
            vec![
                QuestionnaireQuestion::new(
                    "secret",
                    "Secret",
                    "Enter the protected value",
                    Vec::new(),
                )
                .with_free_form(true),
            ],
            Deadline::never(),
        )
        .unwrap();
        let mut state = QuestionnaireState::new(form);
        typed(&mut state, "not-for-logs");
        assert_eq!(state.displayed_draft(), "************");
        assert!(!format!("{state:?}").contains("not-for-logs"));
        let answers = submitted(state.on_key(key(KeyCode::Enter)));
        assert!(!format!("{answers:?}").contains("not-for-logs"));
    }

    #[test]
    fn validation_rejects_unbounded_or_unanswerable_forms() {
        assert!(QuestionnaireForm::new("id", Vec::new(), Deadline::never()).is_err());
        assert!(
            QuestionnaireForm::new(
                "id",
                vec![QuestionnaireQuestion::new(
                    "q",
                    "Question",
                    "No answer surface",
                    Vec::new(),
                )],
                Deadline::never(),
            )
            .is_err()
        );
    }
}
