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

/// Which control owns keyboard input inside the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionnaireFocus {
    /// Choice list or free-form editor.
    Answer,
    /// Navigate to the prior question.
    Back,
    /// Navigate to the next question.
    Next,
    /// Submit every staged answer.
    Submit,
    /// Decline the entire questionnaire.
    Decline,
}

/// Pure reducer state for one questionnaire.
pub struct QuestionnaireState {
    form: QuestionnaireForm,
    current: usize,
    choice_cursor: usize,
    focus: QuestionnaireFocus,
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
            .field("focus", &self.focus)
            .field("staged_answers", &self.staged.len())
            .field("restored", &self.form.restored)
            .finish_non_exhaustive()
    }
}

impl QuestionnaireState {
    /// Starts with no implicit answer or action selected.
    pub fn new(form: QuestionnaireForm) -> Self {
        Self {
            form,
            current: 0,
            choice_cursor: 0,
            focus: QuestionnaireFocus::Answer,
            staged: BTreeMap::new(),
            drafts: BTreeMap::new(),
            error: None,
        }
    }

    /// The immutable projected request.
    pub fn form(&self) -> &QuestionnaireForm {
        &self.form
    }

    /// The visible question.
    pub fn question(&self) -> &QuestionnaireQuestion {
        &self.form.questions[self.current]
    }

    /// Zero-based question position.
    pub fn current_index(&self) -> usize {
        self.current
    }

    /// Highlighted choice position.
    pub fn choice_cursor(&self) -> usize {
        self.choice_cursor
    }

    /// Current keyboard focus.
    pub fn focus(&self) -> QuestionnaireFocus {
        self.focus
    }

    /// Local validation error shown inside the overlay.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the current question has a staged answer.
    pub fn current_is_answered(&self) -> bool {
        self.staged.contains_key(&self.question().id)
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

    /// The staged choice ID for the current question, if any.
    pub fn staged_choice(&self) -> Option<&str> {
        match self.staged.get(&self.question().id) {
            Some(QuestionnaireAnswerValue::Choice(choice)) => Some(choice),
            _ => None,
        }
    }

    /// Handles one key without performing I/O.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<QuestionnaireResolution> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.code == KeyCode::Esc {
            return Some(QuestionnaireResolution::Cancelled);
        }
        match key.code {
            KeyCode::Tab => {
                self.move_focus(false);
                None
            }
            KeyCode::BackTab => {
                self.move_focus(true);
                None
            }
            KeyCode::Enter => self.activate_focus(),
            _ if self.focus == QuestionnaireFocus::Answer => self.edit_answer(key),
            _ => None,
        }
    }

    fn controls(&self) -> Vec<QuestionnaireFocus> {
        let mut controls = vec![QuestionnaireFocus::Answer];
        if self.current > 0 {
            controls.push(QuestionnaireFocus::Back);
        }
        if self.current + 1 < self.form.questions.len() {
            controls.push(QuestionnaireFocus::Next);
        }
        controls.extend([QuestionnaireFocus::Submit, QuestionnaireFocus::Decline]);
        controls
    }

    fn move_focus(&mut self, backwards: bool) {
        let controls = self.controls();
        let current = controls
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        let next = if backwards {
            current.checked_sub(1).unwrap_or(controls.len() - 1)
        } else {
            (current + 1) % controls.len()
        };
        self.focus = controls[next];
        self.error = None;
    }

    fn activate_focus(&mut self) -> Option<QuestionnaireResolution> {
        match self.focus {
            QuestionnaireFocus::Answer => {
                if self.question().choices.is_empty() {
                    self.stage_free_form();
                } else {
                    self.stage_choice(self.choice_cursor);
                }
                None
            }
            QuestionnaireFocus::Back => {
                self.current = self.current.saturating_sub(1);
                self.choice_cursor = 0;
                self.focus = QuestionnaireFocus::Answer;
                self.error = None;
                None
            }
            QuestionnaireFocus::Next => {
                if !self.current_is_answered() {
                    self.error = Some("stage an answer before continuing".to_owned());
                } else {
                    self.current = (self.current + 1).min(self.form.questions.len() - 1);
                    self.choice_cursor = 0;
                    self.focus = QuestionnaireFocus::Answer;
                    self.error = None;
                }
                None
            }
            QuestionnaireFocus::Submit => self.submit(),
            QuestionnaireFocus::Decline => Some(QuestionnaireResolution::Declined),
        }
    }

    fn edit_answer(&mut self, key: KeyEvent) -> Option<QuestionnaireResolution> {
        match key.code {
            KeyCode::Up => {
                let count = self.question().choices.len();
                if count > 0 {
                    self.choice_cursor = self.choice_cursor.checked_sub(1).unwrap_or(count - 1);
                }
            }
            KeyCode::Down => {
                let count = self.question().choices.len();
                if count > 0 {
                    self.choice_cursor = (self.choice_cursor + 1) % count;
                }
            }
            KeyCode::Char(number @ '1'..='9')
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                let index = usize::from(number as u8 - b'1');
                if index < self.question().choices.len() {
                    self.choice_cursor = index;
                    self.stage_choice(index);
                } else {
                    self.insert_free_form(number);
                }
            }
            KeyCode::Char(' ') if !self.question().choices.is_empty() => {
                self.stage_choice(self.choice_cursor);
            }
            KeyCode::Backspace => {
                if self.question().allows_free_form {
                    self.draft_mut().backspace();
                    self.stage_draft_if_nonblank();
                }
            }
            KeyCode::Delete => {
                if self.question().allows_free_form {
                    self.draft_mut().delete();
                    self.stage_draft_if_nonblank();
                }
            }
            KeyCode::Left => {
                if self.question().allows_free_form {
                    self.draft_mut().move_left();
                }
            }
            KeyCode::Right => {
                if self.question().allows_free_form {
                    self.draft_mut().move_right();
                }
            }
            KeyCode::Home => {
                if self.question().allows_free_form {
                    self.draft_mut().move_home();
                }
            }
            KeyCode::End => {
                if self.question().allows_free_form {
                    self.draft_mut().move_end();
                }
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                self.insert_free_form(character);
            }
            _ => {}
        }
        None
    }

    fn insert_free_form(&mut self, character: char) {
        if !self.question().allows_free_form || self.draft_mut().len() >= MAX_FREE_FORM_CHARS {
            return;
        }
        self.draft_mut().insert(character);
        self.stage_draft_if_nonblank();
    }

    /// Inserts pasted text into the free-form draft, flattened to one line.
    ///
    /// Digits and spaces in a paste are draft characters, never choice
    /// shortcuts — only deliberate key presses select answers.
    pub fn paste(&mut self, text: &str) {
        let mut pending_space = false;
        for character in text.chars() {
            if character == '\n' || character.is_whitespace() {
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
        let question = self.form.questions[self.current].id.clone();
        self.drafts.entry(question).or_default()
    }

    fn stage_choice(&mut self, index: usize) {
        let Some(choice) = self.question().choices.get(index) else {
            return;
        };
        self.staged.insert(
            self.question().id.clone(),
            QuestionnaireAnswerValue::Choice(choice.id.clone()),
        );
        self.error = None;
    }

    fn stage_free_form(&mut self) {
        let question_id = self.question().id.clone();
        let value = self
            .drafts
            .get(&question_id)
            .map(Composer::text)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if value.is_empty() {
            self.staged.remove(&question_id);
            self.error = Some("enter an answer before staging it".to_owned());
        } else {
            self.staged
                .insert(question_id, QuestionnaireAnswerValue::FreeForm(value));
            self.error = None;
        }
    }

    fn stage_draft_if_nonblank(&mut self) {
        let question_id = self.question().id.clone();
        let value = self
            .drafts
            .get(&question_id)
            .map(Composer::text)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if value.is_empty() {
            self.staged.remove(&question_id);
        } else {
            self.staged
                .insert(question_id, QuestionnaireAnswerValue::FreeForm(value));
        }
        self.error = None;
    }

    fn submit(&mut self) -> Option<QuestionnaireResolution> {
        let unanswered = self
            .form
            .questions
            .iter()
            .position(|question| !self.staged.contains_key(&question.id));
        if let Some(index) = unanswered {
            self.current = index;
            self.choice_cursor = 0;
            self.focus = QuestionnaireFocus::Answer;
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
                ),
                QuestionnaireQuestion::new("note", "Note", "Anything else?", Vec::new())
                    .with_free_form(true),
            ],
            Deadline::never(),
        )
        .unwrap()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn choice_enter_stages_but_never_implicitly_submits() {
        let mut state = QuestionnaireState::new(form());
        assert_eq!(state.on_key(key(KeyCode::Enter)), None);
        assert_eq!(state.staged_choice(), Some("minimal"));
        assert_eq!(state.current_index(), 0);

        state.on_key(key(KeyCode::Tab));
        assert_eq!(state.focus(), QuestionnaireFocus::Next);
        assert_eq!(state.on_key(key(KeyCode::Enter)), None);
        assert_eq!(state.current_index(), 1);
    }

    #[test]
    fn submission_is_an_explicit_control_and_preserves_question_order() {
        let mut state = QuestionnaireState::new(form());
        state.on_key(key(KeyCode::Char('2')));
        state.on_key(key(KeyCode::Tab));
        state.on_key(key(KeyCode::Enter));
        for character in "ship it".chars() {
            state.on_key(key(KeyCode::Char(character)));
        }
        state.on_key(key(KeyCode::Tab));
        // Last question: Answer -> Back -> Submit.
        state.on_key(key(KeyCode::Tab));
        assert_eq!(state.focus(), QuestionnaireFocus::Submit);
        let resolution = state.on_key(key(KeyCode::Enter)).unwrap();
        let QuestionnaireResolution::Submitted(answers) = resolution else {
            panic!("expected a submitted questionnaire");
        };
        assert_eq!(
            answers
                .iter()
                .map(|answer| answer.question_id.as_str())
                .collect::<Vec<_>>(),
            vec!["design", "note"]
        );
        assert_eq!(
            answers[0].value,
            QuestionnaireAnswerValue::Choice("dense".to_owned())
        );
        assert_eq!(
            answers[1].value,
            QuestionnaireAnswerValue::FreeForm("ship it".to_owned())
        );
    }

    #[test]
    fn decline_and_cancel_are_distinct_typed_outcomes() {
        let mut decline = QuestionnaireState::new(form());
        decline.on_key(key(KeyCode::BackTab));
        assert_eq!(decline.focus(), QuestionnaireFocus::Decline);
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
        for character in "not-for-logs".chars() {
            state.on_key(key(KeyCode::Char(character)));
        }
        assert_eq!(state.displayed_draft(), "************");
        assert!(!format!("{state:?}").contains("not-for-logs"));
        state.on_key(key(KeyCode::Tab));
        let QuestionnaireResolution::Submitted(answers) =
            state.on_key(key(KeyCode::Enter)).unwrap()
        else {
            panic!("expected submission");
        };
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
