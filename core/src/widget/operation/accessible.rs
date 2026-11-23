//! Describe widgets to assistive technology.
//!
//! A widget tells what it is in its `operate`, next to what it already
//! tells there (its text, its focus): one call of
//! [`Operation::accessible`](super::Operation::accessible) with an
//! [`Accessible`]. A shell collects these calls into the tree a screen
//! reader walks. The children of the widget (what it visits in the
//! `traverse` that follows) become the children of its node.

/// What a widget is, to a person who cannot see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    /// Something that is pressed.
    Button,
    /// A box that is ticked or not (or in part).
    CheckBox,
    /// A switch that is on or off.
    Switch,
    /// One choice among a few, of which one is chosen.
    RadioButton,
    /// A list that opens to choose one of its options.
    ComboBox,
    /// A bar that shows how far something is.
    ProgressBar,
    /// A field text is typed in.
    TextInput,
    /// A field a secret is typed in (a password): what is typed is not
    /// told.
    SecureTextInput,
    /// A handle on a rail that chooses a number between two ends.
    Slider,
}

/// Whether something that can be ticked is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Toggled {
    /// It is not.
    No,
    /// It is.
    Yes,
    /// It is in part.
    Mixed,
}

impl From<bool> for Toggled {
    fn from(toggled: bool) -> Self {
        if toggled { Toggled::Yes } else { Toggled::No }
    }
}

/// The description of a widget for assistive technology.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Accessible<'a> {
    /// What the widget is.
    pub role: Role,
    /// Its name: the text a person would call it by. A widget without one
    /// is named after the text of its children.
    pub label: Option<&'a str>,
    /// What it shows while it is empty (a field) or nothing is chosen (a
    /// list): an example, not a name. A widget without a name is called by
    /// it.
    pub placeholder: Option<&'a str>,
    /// Its value as text: the chosen option of a list.
    pub value: Option<&'a str>,
    /// Whether it is ticked, for what can be.
    pub toggled: Option<Toggled>,
    /// Its value as a number with the least and the most it can be.
    pub range: Option<(f32, f32, f32)>,
    /// How far one step moves the number of [`range`](Self::range).
    pub step: Option<f32>,
    /// Whether it can be used.
    pub is_enabled: bool,
}

impl<'a> Accessible<'a> {
    /// Creates the description of an enabled widget with the given [`Role`].
    pub fn new(role: Role) -> Self {
        Self {
            role,
            label: None,
            placeholder: None,
            value: None,
            toggled: None,
            range: None,
            step: None,
            is_enabled: true,
        }
    }

    /// Sets the name of the widget.
    pub fn label(mut self, label: impl Into<Option<&'a str>>) -> Self {
        self.label = label.into();
        self
    }

    /// Sets what the widget shows while it is empty.
    pub fn placeholder(
        mut self,
        placeholder: impl Into<Option<&'a str>>,
    ) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Sets the value of the widget, as text.
    pub fn value(mut self, value: impl Into<Option<&'a str>>) -> Self {
        self.value = value.into();
        self
    }

    /// Sets whether the widget is ticked.
    pub fn toggled(mut self, toggled: impl Into<Toggled>) -> Self {
        self.toggled = Some(toggled.into());
        self
    }

    /// Sets the value of the widget as a number, with the least and the
    /// most it can be.
    pub fn range(mut self, value: f32, min: f32, max: f32) -> Self {
        self.range = Some((value, min, max));
        self
    }

    /// Sets how far one step moves the number of the widget.
    pub fn step(mut self, step: f32) -> Self {
        self.step = Some(step);
        self
    }

    /// Sets whether the widget can be used.
    pub fn enabled(mut self, is_enabled: bool) -> Self {
        self.is_enabled = is_enabled;
        self
    }
}
