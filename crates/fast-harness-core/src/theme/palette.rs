//! The colour roles a front end draws with: one [`Palette`] field per role,
//! and the same names as the `colors` keys of a theme file (`docs/themes.md`).

use super::Rgb;

macro_rules! palette {
    ($( $(#[doc = $doc:literal])* $role:ident ),* $(,)?) => {
        /// The colour roles a front end draws with.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct Palette {
            $( $(#[doc = $doc])* pub $role: Rgb, )*
        }

        /// Every role name in declaration order: the `colors` keys of a theme file.
        pub const ROLES: &[&str] = &[$(stringify!($role)),*];

        impl Palette {
            /// Every role set to `c`.
            pub fn filled(c: Rgb) -> Palette {
                Palette { $($role: c),* }
            }

            /// Every role with its name, in declaration order.
            pub fn roles(&self) -> Vec<(&'static str, Rgb)> {
                vec![$((stringify!($role), self.$role)),*]
            }

            /// The colour of the role `name`, `None` for an unknown name.
            pub fn get(&self, name: &str) -> Option<Rgb> {
                match name {
                    $(stringify!($role) => Some(self.$role),)*
                    _ => None,
                }
            }

            /// Sets the role `name`; `false` for an unknown name.
            pub fn set(&mut self, name: &str, c: Rgb) -> bool {
                match name {
                    $(stringify!($role) => { self.$role = c; true })*
                    _ => false,
                }
            }

            /// Every role passed through `f`.
            pub fn map(&self, f: impl Fn(Rgb) -> Rgb) -> Palette {
                Palette { $($role: f(self.$role)),* }
            }
        }
    };
}

palette! {
    /// Window, transcript and page background; the start of the background gradient.
    bg,
    /// The end of the background gradient (`bg` at the top left, `bg_end` at
    /// the bottom right); text meets the same contrast on it as on `bg`.
    bg_end,
    /// Sidebar, header and status bars, cards and the composer.
    surface,
    /// Input fields, dropdowns and buttons at rest.
    raised,
    /// A hovered row or control.
    hover,
    /// A selected row or list item.
    selection,
    /// The user's message bubble.
    bubble,
    /// Code spans and code blocks.
    code,
    /// A surface tinted with the accent: the active navigation item, a
    /// highlighted card, a callout. Body text meets 7:1 on it.
    accent_surface,
    /// Decorative separators; no contrast requirement.
    line,
    /// Borders of inputs, dropdowns and buttons: 3:1 against `bg`, `bg_end`,
    /// `surface` and `raised`.
    border,
    /// Body text: 7:1 and APCA Lc 75 on every background role.
    text,
    /// Secondary text: captions, timings, thinking, hints.
    dim,
    /// Links, the focus ring, active markers, the primary button and the
    /// start of accent gradients.
    accent,
    /// The second accent: the end of accent gradients (primary button,
    /// active tab, logo) and secondary highlights; as readable as `accent`.
    accent2,
    /// Text and icons on `accent`, on `accent2` and on a gradient between them.
    on_accent,
    /// A coloured glow around the focused or primary element and behind
    /// decorative gradients; drawn with transparency, no contrast requirement.
    glow,
    /// Drop shadows of cards, popups and menus; drawn with transparency, no
    /// contrast requirement.
    shadow,
    /// Error text and marks (always with a word or a mark).
    error,
    /// Warning text and marks (always with a word or a mark).
    warning,
    /// Success text and marks (always with a word or a mark).
    success,
    /// Information text and marks (always with a word or a mark).
    info,
}
