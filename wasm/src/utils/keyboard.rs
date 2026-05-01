use strum::{AsRefStr, Display, EnumString};

/// Share key identifier between App (Escape) and InputArea (Enter)
#[derive(Debug, Display, EnumString, AsRefStr)]
#[strum(serialize_all = "PascalCase")]
pub(crate) enum KeyboardId {
    Escape,
    Enter,
}
