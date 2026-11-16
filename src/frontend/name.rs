//! 名前の構造と、コンパイラが生成する名前の種類。
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameKind {
    User,
    Associated(Box<Name>, Box<Name>),
    Qualified(String, Box<Name>),
    Instance(usize),
    Implementation(Box<Name>, usize),
    ImplementationMethod(Box<Name>, Box<Name>),
    Structure(Box<Name>, StructureStyle),
    Supertrait(Box<Name>),
    DefaultMethod(Box<Name>, Box<Name>),
    TraitAccessor(Box<Name>),
    Hygienic(String, usize),
    EffectParameter(Box<Name>, usize),
    Capability(Box<Name>, bool),
    Temporary(String, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StructureStyle {
    Record,
    Tuple,
}

/// 綴りは表示に使い、名前の同一性には種類と構造も含める。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name {
    spelling: String,
    pub kind: NameKind,
}

impl Name {
    pub fn as_str(&self) -> &str {
        &self.spelling
    }

    pub fn associated(owner: Name, member: Name) -> Self {
        if let NameKind::Qualified(module, local) = &owner.kind {
            return Self::qualified(module, Self::associated(*local.clone(), member));
        }
        Self {
            spelling: format!("{owner}::{member}"),
            kind: NameKind::Associated(Box::new(owner), Box::new(member)),
        }
    }

    pub fn qualified(module: &str, member: Name) -> Self {
        if let Some((first, rest)) = module.split_once('.') {
            return Self::qualified(first, Self::qualified(rest, member));
        }
        if module.is_empty() {
            member
        } else {
            Self {
                spelling: format!("{module}.{member}"),
                kind: NameKind::Qualified(module.into(), Box::new(member)),
            }
        }
    }

    pub fn instance(index: usize) -> Self {
        Self {
            spelling: format!("inst#{index}"),
            kind: NameKind::Instance(index),
        }
    }

    pub fn implementation(trait_name: Name, index: usize) -> Self {
        Self {
            spelling: format!("{trait_name}#impl{index}"),
            kind: NameKind::Implementation(Box::new(trait_name), index),
        }
    }

    pub fn implementation_method(implementation: Name, method: Name) -> Self {
        if let NameKind::Qualified(module, local) = &implementation.kind {
            return Self::qualified(module, Self::implementation_method(*local.clone(), method));
        }
        Self {
            spelling: format!("{implementation}#{method}"),
            kind: NameKind::ImplementationMethod(Box::new(implementation), Box::new(method)),
        }
    }

    pub fn structure(owner: Name) -> Self {
        if let NameKind::Qualified(module, local) = &owner.kind {
            return Self::qualified(module, Self::structure(*local.clone()));
        }
        Self {
            spelling: format!("{owner}{{}}"),
            kind: NameKind::Structure(Box::new(owner), StructureStyle::Record),
        }
    }

    pub fn tuple_structure(owner: Name) -> Self {
        if let NameKind::Qualified(module, local) = &owner.kind {
            return Self::qualified(module, Self::tuple_structure(*local.clone()));
        }
        Self {
            spelling: format!("{owner}{{}}"),
            kind: NameKind::Structure(Box::new(owner), StructureStyle::Tuple),
        }
    }

    pub fn supertrait(trait_name: Name) -> Self {
        Self {
            spelling: format!("super#{trait_name}"),
            kind: NameKind::Supertrait(Box::new(trait_name)),
        }
    }

    pub fn default_method(trait_name: Name, method: Name) -> Self {
        if let NameKind::Qualified(module, local) = &trait_name.kind {
            return Self::qualified(module, Self::default_method(*local.clone(), method));
        }
        Self {
            spelling: format!("{trait_name}#default#{method}"),
            kind: NameKind::DefaultMethod(Box::new(trait_name), Box::new(method)),
        }
    }

    pub fn trait_accessor(trait_name: Name) -> Self {
        if let NameKind::Qualified(module, local) = &trait_name.kind {
            return Self::qualified(module, Self::trait_accessor(*local.clone()));
        }
        Self {
            spelling: format!("{trait_name}#get"),
            kind: NameKind::TraitAccessor(Box::new(trait_name)),
        }
    }

    pub fn hygienic(hint: &str, index: usize) -> Self {
        Self {
            spelling: format!("{hint}#{index}"),
            kind: NameKind::Hygienic(hint.into(), index),
        }
    }

    pub fn local(&self) -> &Name {
        match &self.kind {
            NameKind::Qualified(_, member) => member.local(),
            _ => self,
        }
    }

    pub fn effect_parameter(name: Name, index: usize) -> Self {
        Self {
            spelling: format!("{name}#{index}"),
            kind: NameKind::EffectParameter(Box::new(name), index),
        }
    }

    pub fn is_effect_parameter(&self) -> bool {
        matches!(self.kind, NameKind::EffectParameter(..))
    }

    pub fn capability(trait_name: Name, type_parameter: bool) -> Self {
        let prefix = if type_parameter { "captype" } else { "cap" };
        Self {
            spelling: format!("{prefix}#{trait_name}"),
            kind: NameKind::Capability(Box::new(trait_name), type_parameter),
        }
    }

    pub fn temporary(hint: &str, index: usize) -> Self {
        Self {
            spelling: format!("{hint}#{index}"),
            kind: NameKind::Temporary(hint.into(), index),
        }
    }

    pub fn capability_trait(&self) -> Option<&Name> {
        match &self.kind {
            NameKind::Capability(trait_name, false) => Some(trait_name),
            _ => None,
        }
    }

    pub fn structure_owner(&self) -> Option<Name> {
        match &self.kind {
            NameKind::Qualified(module, local) => local
                .structure_owner()
                .map(|owner| Self::qualified(module, owner)),
            NameKind::Structure(owner, _) => Some(*owner.clone()),
            _ => None,
        }
    }

    pub fn structure_style(&self) -> Option<StructureStyle> {
        match &self.local().kind {
            NameKind::Structure(_, style) => Some(*style),
            _ => None,
        }
    }

    pub fn is_instance(&self) -> bool {
        matches!(self.kind, NameKind::Instance(_))
    }
    pub fn is_implementation(&self) -> bool {
        matches!(self.local().kind, NameKind::Implementation(..))
    }
    pub fn is_supertrait(&self) -> bool {
        matches!(self.kind, NameKind::Supertrait(_))
    }
}

impl From<String> for Name {
    fn from(spelling: String) -> Self {
        if let Some((module, member)) = spelling.split_once('.') {
            return Self::qualified(module, member.into());
        }
        if let Some((owner, member)) = spelling.rsplit_once("::") {
            return Self::associated(owner.into(), member.into());
        }
        Self {
            spelling,
            kind: NameKind::User,
        }
    }
}
impl From<&str> for Name {
    fn from(name: &str) -> Self {
        Self::from(name.to_string())
    }
}
impl From<&Name> for Name {
    fn from(name: &Name) -> Self {
        name.clone()
    }
}
impl From<&String> for Name {
    fn from(name: &String) -> Self {
        Self::from(name.as_str())
    }
}
impl std::ops::Deref for Name {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self == &Name::from(other)
    }
}
impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self == &Name::from(*other)
    }
}
impl PartialEq<String> for Name {
    fn eq(&self, other: &String) -> bool {
        self == &Name::from(other)
    }
}
impl PartialEq<Name> for str {
    fn eq(&self, other: &Name) -> bool {
        Name::from(self) == *other
    }
}
impl PartialEq<Name> for &str {
    fn eq(&self, other: &Name) -> bool {
        Name::from(*self) == *other
    }
}
impl PartialEq<Name> for String {
    fn eq(&self, other: &Name) -> bool {
        Name::from(self.as_str()) == *other
    }
}
impl From<Name> for String {
    fn from(name: Name) -> Self {
        name.spelling
    }
}
impl From<Name> for std::rc::Rc<str> {
    fn from(name: Name) -> Self {
        name.spelling.into()
    }
}
