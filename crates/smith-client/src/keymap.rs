//! Descriptive bindings shared by help surfaces and checked by the TUI tests.
//!
//! This table does not dispatch input. Prompt guards and overlay precedence
//! remain the terminal client's responsibility. A help row can describe
//! several cases with different contexts or effects.

/// A terminal-independent named key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// One character key.
    Char(char),
    /// The return key.
    Enter,
    /// The forward tab key.
    Tab,
    /// The reverse tab key, usually reported for Shift+Tab.
    BackTab,
    /// The escape key.
    Escape,
    /// The upward arrow.
    Up,
    /// The downward arrow.
    Down,
    /// The home key.
    Home,
    /// The end key.
    End,
    /// The previous-page key.
    PageUp,
    /// The next-page key.
    PageDown,
}

/// Modifiers used by the listed bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    /// No modifier is held.
    None,
    /// Control is held.
    Control,
    /// Alt is held.
    Alt,
    /// Shift is held.
    Shift,
}

/// A named key and its modifier, without a terminal-library dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    /// The key being pressed.
    pub key: Key,
    /// The modifier held with the key.
    pub modifier: Modifier,
}

impl KeyChord {
    /// Constructs one terminal-independent chord.
    pub const fn new(key: Key, modifier: Modifier) -> Self {
        Self { key, modifier }
    }
}

/// Direction in a draft, history, delegated-agent list, or transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Move backward, upward, or toward older entries.
    Previous,
    /// Move forward, downward, or toward newer entries.
    Next,
}

/// The first or last position of a line, draft, or transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The first position.
    Start,
    /// The last position.
    End,
}

/// A chord, typed text, wheel input, or ordered sequence of those inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    /// One key press with a modifier.
    Chord(KeyChord),
    /// Literal characters typed in order.
    Text(&'static str),
    /// One mouse-wheel report in the given direction.
    MouseWheel(Direction),
    /// Inputs delivered consecutively; repeated chords retain their order.
    Sequence(&'static [KeyInput]),
}

/// The input surface and state in which a listed effect applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    /// An ordinary nonempty draft while no turn is serving.
    Idle,
    /// An ordinary nonempty draft while a turn is serving.
    Working,
    /// An empty draft while no turn is serving.
    EmptyIdleDraft,
    /// An empty draft, whether idle or working.
    EmptyDraft,
    /// A nonempty composer without an overlay, whether idle or working.
    AnyComposer,
    /// A cursor within a multiline draft.
    DraftLine,
    /// The cursor is on the first draft line, with earlier input in history.
    DraftFirstLine,
    /// The cursor is on the last recalled line, with a newer draft to restore.
    DraftLastLine,
    /// The last draft line with delegated agents after the newest history entry.
    DelegatedAgents,
    /// A delegated agent is being inspected at the first or last draft line.
    InspectedAgent,
    /// An explicit queued turn can be restored for editing.
    QueuedTurn,
    /// A serving turn has a foreground shell call; the host owns the process.
    ForegroundShell,
    /// Transcript following is paused, whether idle or working.
    PausedOutput,
    /// An empty draft while transcript following is paused, idle or working.
    EmptyPausedOutput,
    /// An approval owns input after its consequential-input guard has elapsed.
    Approval,
    /// A confirmation owns input after its consequential-input guard has elapsed.
    Confirm,
    /// The command palette owns input.
    CommandPalette,
    /// Input history search owns input and has a matching entry.
    HistorySearch,
    /// The composer cursor is at the start of a reference token.
    TokenBoundary,
    /// A prepared local shell command is ready for a leading bang and Enter.
    ShellCommand,
}

/// Observable effects; TUI checks must handle every variant exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEffect {
    /// Submit a whole ordinary user turn.
    SendTask,
    /// Submit ordinary input to the currently serving turn.
    Steer,
    /// Queue an explicit ordinary turn after the serving turn.
    QueueTurn,
    /// Select the next configured profile.
    NextProfile,
    /// Select the previous configured profile.
    PreviousProfile,
    /// Expand or fold work detail.
    ToggleDetail,
    /// Interrupt work or dismiss the currently active surface.
    InterruptOrClose,
    /// Insert a newline without submission.
    InsertNewline,
    /// Move to an adjacent draft line.
    MoveDraftLine(Direction),
    /// Recall earlier input or restore the newer draft.
    BrowseHistory(Direction),
    /// Move inspection through the delegated-agent list.
    BrowseAgents(Direction),
    /// Move to a draft boundary.
    DraftEdge(Edge),
    /// Move to the oldest or newest transcript output.
    OutputEdge(Edge),
    /// Move to a boundary of the current draft line.
    LineEdge(Edge),
    /// Move by one word.
    MoveWord(Direction),
    /// Delete the word preceding the cursor.
    DeleteWordLeft,
    /// Delete draft text before the cursor on the current line.
    DeleteToLineStart,
    /// Delete draft text after the cursor on the current line.
    DeleteToLineEnd,
    /// Restore the newest explicit queued turn to the composer.
    EditQueuedTurn,
    /// Ask the host to background its foreground shell process.
    BackgroundShell,
    /// Open slash-command completion.
    OpenCommandPalette,
    /// Open the ephemeral shortcuts panel.
    ShowShortcuts,
    /// Scroll the transcript without editing the draft.
    ScrollTranscript(Direction),
    /// Resume following the newest transcript output.
    FollowNewest,
    /// Open reverse input-history search.
    SearchHistory,
    /// Restore the current history match without submitting it.
    RestoreHistoryMatch,
    /// Close history search and restore the original draft.
    CloseHistorySearch,
    /// Save the current draft in history and clear the composer.
    StashDraft,
    /// Force exit after consecutive Ctrl+C presses within one second.
    Quit,
    /// Open completion containing files and agents.
    CompleteReferences,
    /// Submit an escaped leading marker as ordinary text.
    SendLiteral(char),
    /// Enter the local shell composer mode.
    ShellMode,
    /// Submit a prepared local shell command to the host.
    RunShell,
}

/// One concrete input and effect within a help row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingCase {
    /// The state in which this case applies.
    pub context: KeyContext,
    /// The chord, text, wheel report, or sequence to deliver.
    pub input: KeyInput,
    /// The resulting observable effect.
    pub effect: KeyEffect,
}

impl BindingCase {
    const fn new(context: KeyContext, input: KeyInput, effect: KeyEffect) -> Self {
        Self {
            context,
            input,
            effect,
        }
    }

    const fn key(context: KeyContext, key: Key, modifier: Modifier, effect: KeyEffect) -> Self {
        Self::new(
            context,
            KeyInput::Chord(KeyChord::new(key, modifier)),
            effect,
        )
    }
}

/// One help row, in display order, with all the cases described by its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    /// Exact existing key label shown in help.
    pub label: &'static str,
    /// Exact existing help description.
    pub description: &'static str,
    /// Concrete alternatives and contextual meanings described by this row.
    pub cases: &'static [BindingCase],
}

use Direction::{Next, Previous};
use Edge::{End, Start};
use KeyContext::{
    AnyComposer, Approval, CommandPalette, Confirm, DelegatedAgents, DraftFirstLine, DraftLastLine,
    DraftLine, EmptyDraft, EmptyIdleDraft, ForegroundShell, HistorySearch, Idle, InspectedAgent,
    PausedOutput, QueuedTurn, ShellCommand, TokenBoundary, Working,
};
use KeyEffect::{
    BackgroundShell, BrowseAgents, BrowseHistory, CloseHistorySearch, CompleteReferences,
    DeleteToLineEnd, DeleteToLineStart, DeleteWordLeft, DraftEdge, EditQueuedTurn, FollowNewest,
    InsertNewline, InterruptOrClose, LineEdge, MoveDraftLine, MoveWord, NextProfile,
    OpenCommandPalette, OutputEdge, PreviousProfile, QueueTurn, Quit, RestoreHistoryMatch,
    RunShell, ScrollTranscript, SearchHistory, SendLiteral, SendTask, ShellMode, ShowShortcuts,
    StashDraft, Steer, ToggleDetail,
};
use Modifier::{Alt, Control, None, Shift};

/// The sole definition of the ordered `/help` and shortcuts key rows.
pub const KEY_BINDINGS: &[KeyBinding] = &[
    KeyBinding {
        label: "Enter",
        description: "Send a task",
        cases: &[BindingCase::key(Idle, Key::Enter, None, SendTask)],
    },
    KeyBinding {
        label: "Enter while working",
        description: "Send now",
        cases: &[BindingCase::key(Working, Key::Enter, None, Steer)],
    },
    KeyBinding {
        label: "Tab while working",
        description: "Queue for after this turn",
        cases: &[BindingCase::key(Working, Key::Tab, None, QueueTurn)],
    },
    KeyBinding {
        label: "Tab when idle",
        description: "Next profile (empty draft)",
        cases: &[BindingCase::key(
            EmptyIdleDraft,
            Key::Tab,
            None,
            NextProfile,
        )],
    },
    KeyBinding {
        label: "Shift+Tab when idle",
        description: "Previous profile (empty draft)",
        cases: &[BindingCase::key(
            EmptyIdleDraft,
            Key::BackTab,
            Shift,
            PreviousProfile,
        )],
    },
    KeyBinding {
        label: "Ctrl+O",
        description: "Expand or fold detail",
        cases: &[
            BindingCase::key(AnyComposer, Key::Char('o'), Control, ToggleDetail),
            BindingCase::key(Approval, Key::Char('o'), Control, ToggleDetail),
            BindingCase::key(Confirm, Key::Char('o'), Control, ToggleDetail),
        ],
    },
    KeyBinding {
        label: "Esc",
        description: "Interrupt or close",
        cases: &[
            BindingCase::key(Idle, Key::Escape, None, InterruptOrClose),
            BindingCase::key(Working, Key::Escape, None, InterruptOrClose),
            BindingCase::key(CommandPalette, Key::Escape, None, InterruptOrClose),
            BindingCase::key(Approval, Key::Escape, None, InterruptOrClose),
            BindingCase::key(Confirm, Key::Escape, None, InterruptOrClose),
            BindingCase::key(InspectedAgent, Key::Escape, None, InterruptOrClose),
        ],
    },
    KeyBinding {
        label: "Shift+Enter or Alt+Enter",
        description: "Insert a newline",
        cases: &[
            BindingCase::key(AnyComposer, Key::Enter, Shift, InsertNewline),
            BindingCase::key(AnyComposer, Key::Enter, Alt, InsertNewline),
        ],
    },
    KeyBinding {
        label: "\\ then Enter",
        description: "Replace the backslash with a newline",
        cases: &[BindingCase::new(
            AnyComposer,
            KeyInput::Sequence(&[
                KeyInput::Text("\\"),
                KeyInput::Chord(KeyChord::new(Key::Enter, None)),
            ]),
            InsertNewline,
        )],
    },
    KeyBinding {
        label: "Up / Down",
        description: "Move between draft lines; browse history and delegated agents from the first or last line",
        cases: &[
            BindingCase::key(DraftLine, Key::Up, None, MoveDraftLine(Previous)),
            BindingCase::key(DraftLine, Key::Down, None, MoveDraftLine(Next)),
            BindingCase::key(DraftFirstLine, Key::Up, None, BrowseHistory(Previous)),
            BindingCase::key(DraftLastLine, Key::Down, None, BrowseHistory(Next)),
            BindingCase::key(DelegatedAgents, Key::Down, None, BrowseAgents(Next)),
            BindingCase::key(InspectedAgent, Key::Up, None, BrowseAgents(Previous)),
        ],
    },
    KeyBinding {
        label: "Home / End",
        description: "Go to draft start or end; when empty, go to oldest or newest output",
        cases: &[
            BindingCase::key(AnyComposer, Key::Home, None, DraftEdge(Start)),
            BindingCase::key(AnyComposer, Key::End, None, DraftEdge(End)),
            BindingCase::key(
                KeyContext::EmptyPausedOutput,
                Key::Home,
                None,
                OutputEdge(Start),
            ),
            BindingCase::key(
                KeyContext::EmptyPausedOutput,
                Key::End,
                None,
                OutputEdge(End),
            ),
        ],
    },
    KeyBinding {
        label: "Ctrl+A / Ctrl+E",
        description: "Go to line start or end",
        cases: &[
            BindingCase::key(DraftLine, Key::Char('a'), Control, LineEdge(Start)),
            BindingCase::key(DraftLine, Key::Char('e'), Control, LineEdge(End)),
        ],
    },
    KeyBinding {
        label: "Alt+B / Alt+F",
        description: "Move one word left or right",
        cases: &[
            BindingCase::key(DraftLine, Key::Char('b'), Alt, MoveWord(Previous)),
            BindingCase::key(DraftLine, Key::Char('f'), Alt, MoveWord(Next)),
        ],
    },
    KeyBinding {
        label: "Ctrl+W",
        description: "Delete the word to the left",
        cases: &[BindingCase::key(
            DraftLine,
            Key::Char('w'),
            Control,
            DeleteWordLeft,
        )],
    },
    KeyBinding {
        label: "Ctrl+U",
        description: "Delete to line start",
        cases: &[BindingCase::key(
            DraftLine,
            Key::Char('u'),
            Control,
            DeleteToLineStart,
        )],
    },
    KeyBinding {
        label: "Ctrl+K",
        description: "Delete to line end",
        cases: &[BindingCase::key(
            DraftLine,
            Key::Char('k'),
            Control,
            DeleteToLineEnd,
        )],
    },
    KeyBinding {
        label: "Alt+Up",
        description: "Edit the newest queued task",
        cases: &[BindingCase::key(QueuedTurn, Key::Up, Alt, EditQueuedTurn)],
    },
    KeyBinding {
        label: "Ctrl+B",
        description: "Move a running shell command to the background",
        cases: &[BindingCase::key(
            ForegroundShell,
            Key::Char('b'),
            Control,
            BackgroundShell,
        )],
    },
    KeyBinding {
        label: "Ctrl+P",
        description: "Open the command palette",
        cases: &[BindingCase::key(
            AnyComposer,
            Key::Char('p'),
            Control,
            OpenCommandPalette,
        )],
    },
    KeyBinding {
        label: "? on an empty draft",
        description: "Show shortcuts",
        cases: &[
            BindingCase::key(EmptyDraft, Key::Char('?'), None, ShowShortcuts),
            BindingCase::key(EmptyDraft, Key::Char('?'), Shift, ShowShortcuts),
        ],
    },
    KeyBinding {
        label: "PageUp / PageDown",
        description: "Scroll the transcript",
        cases: &[
            BindingCase::key(PausedOutput, Key::PageUp, None, ScrollTranscript(Previous)),
            BindingCase::key(PausedOutput, Key::PageDown, None, ScrollTranscript(Next)),
            BindingCase::key(Approval, Key::PageUp, None, ScrollTranscript(Previous)),
            BindingCase::key(Approval, Key::PageDown, None, ScrollTranscript(Next)),
        ],
    },
    KeyBinding {
        label: "Mouse wheel",
        description: "Scroll the transcript",
        cases: &[
            BindingCase::new(
                PausedOutput,
                KeyInput::MouseWheel(Previous),
                ScrollTranscript(Previous),
            ),
            BindingCase::new(
                PausedOutput,
                KeyInput::MouseWheel(Next),
                ScrollTranscript(Next),
            ),
            BindingCase::new(
                Approval,
                KeyInput::MouseWheel(Previous),
                ScrollTranscript(Previous),
            ),
            BindingCase::new(Approval, KeyInput::MouseWheel(Next), ScrollTranscript(Next)),
        ],
    },
    KeyBinding {
        label: "Ctrl+L",
        description: "Follow the newest output",
        cases: &[
            BindingCase::key(PausedOutput, Key::Char('l'), Control, FollowNewest),
            BindingCase::key(Approval, Key::Char('l'), Control, FollowNewest),
        ],
    },
    KeyBinding {
        label: "Ctrl+R",
        description: "Search input history; Enter restores, Esc closes",
        cases: &[
            BindingCase::key(AnyComposer, Key::Char('r'), Control, SearchHistory),
            BindingCase::key(HistorySearch, Key::Enter, None, RestoreHistoryMatch),
            BindingCase::key(HistorySearch, Key::Escape, None, CloseHistorySearch),
        ],
    },
    KeyBinding {
        label: "Ctrl+C",
        description: "Save the draft in history and clear it",
        cases: &[
            BindingCase::key(AnyComposer, Key::Char('c'), Control, StashDraft),
            BindingCase::key(Approval, Key::Char('c'), Control, StashDraft),
            BindingCase::key(Confirm, Key::Char('c'), Control, StashDraft),
        ],
    },
    KeyBinding {
        label: "Ctrl+C twice",
        description: "Exit (press twice within one second)",
        cases: &[
            BindingCase::new(AnyComposer, CTRL_C_TWICE, Quit),
            BindingCase::new(Approval, CTRL_C_TWICE, Quit),
            BindingCase::new(Confirm, CTRL_C_TWICE, Quit),
        ],
    },
    KeyBinding {
        label: "@",
        description: "Complete files and agents; @@ sends a literal @",
        cases: &[
            BindingCase::new(TokenBoundary, KeyInput::Text("@"), CompleteReferences),
            BindingCase::new(
                EmptyIdleDraft,
                KeyInput::Sequence(&[
                    KeyInput::Text("@@"),
                    KeyInput::Chord(KeyChord::new(Key::Enter, None)),
                ]),
                SendLiteral('@'),
            ),
        ],
    },
    KeyBinding {
        label: "!",
        description: "Run a local shell command; !! sends a literal !",
        cases: &[
            BindingCase::new(EmptyDraft, KeyInput::Text("!"), ShellMode),
            BindingCase::new(
                ShellCommand,
                KeyInput::Sequence(&[
                    KeyInput::Text("!"),
                    KeyInput::Chord(KeyChord::new(Key::Enter, None)),
                ]),
                RunShell,
            ),
            BindingCase::new(
                EmptyIdleDraft,
                KeyInput::Sequence(&[
                    KeyInput::Text("!!"),
                    KeyInput::Chord(KeyChord::new(Key::Enter, None)),
                ]),
                SendLiteral('!'),
            ),
        ],
    },
    KeyBinding {
        label: "//",
        description: "Send text with a leading slash",
        cases: &[BindingCase::new(
            EmptyIdleDraft,
            KeyInput::Sequence(&[
                KeyInput::Text("//"),
                KeyInput::Chord(KeyChord::new(Key::Enter, None)),
            ]),
            SendLiteral('/'),
        )],
    },
];

const CTRL_C_TWICE: KeyInput = KeyInput::Sequence(&[
    KeyInput::Chord(KeyChord::new(Key::Char('c'), Control)),
    KeyInput::Chord(KeyChord::new(Key::Char('c'), Control)),
]);
