(defmacrocatalog pleme-derives
  :entries (
    (
      :crate-name "pleme-getter-derive"
      :description "Per-field inherent getter: pub fn <field>(&self) -> &<Type>."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-field-getter
      :kind per-field
      :spec (
        :trait-name "GetterAll"
        :target named-struct
        :per-field-template "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }"
      ))
    (
      :crate-name "pleme-builder-derive"
      :description "Per-field fluent builder: pub fn with_<field>(mut self, v: <Type>) -> Self."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-field-with-builder
      :kind per-field
      :spec (
        :trait-name "WithBuilder"
        :target named-struct
        :per-field-template "pub fn #method_ident(mut self, v: #field_ty) -> Self { self.#field_name = v; self }"
        :method-name-template "with_{}"
      ))
    (
      :crate-name "pleme-setter-derive"
      :description "Per-field inherent setter: pub fn set_<field>(&mut self, v: <Type>)."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-field-setter
      :kind per-field
      :spec (
        :trait-name "SetterAll"
        :target named-struct
        :per-field-template "pub fn #method_ident(&mut self, v: #field_ty) { self.#field_name = v; }"
        :method-name-template "set_{}"
      ))
    (
      :crate-name "pleme-isvariant-derive"
      :description "Per-variant matches-style predicate: pub fn is_<variant>(&self) -> bool."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-variant-is-variant
      :kind per-variant
      :spec (
        :trait-name "IsVariant"
        :variant-shape any
        :per-variant-template "pub fn #method_ident(&self) -> bool { matches!(self, #variant_shape_arm) }"
        :method-name-template "is_{}"
      ))
    (
      :crate-name "pleme-asmut-derive"
      :description "Per-field mutable getter: pub fn <field>_mut(&mut self) -> &mut <Type>."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-field-as-mut
      :kind per-field
      :spec (
        :trait-name "AsMutAll"
        :target named-struct
        :per-field-template "pub fn #method_ident(&mut self) -> &mut #field_ty { &mut self.#field_name }"
        :method-name-template "{}_mut"
      ))
    (
      :crate-name "pleme-replace-derive"
      :description "Per-field swap via std::mem::replace."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-field-replace
      :kind per-field
      :spec (
        :trait-name "ReplaceAll"
        :target named-struct
        :per-field-template "pub fn #method_ident(&mut self, v: #field_ty) -> #field_ty { ::std::mem::replace(&mut self.#field_name, v) }"
        :method-name-template "replace_{}"
      ))
    (
      :crate-name "pleme-take-derive"
      :description "Per-field steal via std::mem::take."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-field-take
      :kind per-field
      :spec (
        :trait-name "TakeAll"
        :target named-struct
        :per-field-template "pub fn #method_ident(&mut self) -> #field_ty where #field_ty: ::std::default::Default { ::std::mem::take(&mut self.#field_name) }"
        :method-name-template "take_{}"
      ))
    (
      :crate-name "pleme-implfrom-derive"
      :description "Newtype-bidirectional From: impl From<Inner> for Wrapper + From<Wrapper> for Inner."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-impl-from
      :kind newtype
      :spec (
        :trait-name "ImplFrom"
        :target tuple
        :impl-template "impl ::std::convert::From<#inner_ty> for #self_name { fn from(v: #inner_ty) -> Self { Self(v) } } impl ::std::convert::From<#self_name> for #inner_ty { fn from(w: #self_name) -> Self { w.0 } }"
      ))
    (
      :crate-name "pleme-asref-derive"
      :description "Newtype AsRef: impl AsRef<Inner> for Wrapper."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-as-ref
      :kind newtype
      :spec (
        :trait-name "AsRefNewtype"
        :target tuple
        :impl-template "impl ::std::convert::AsRef<#inner_ty> for #self_name { fn as_ref(&self) -> &#inner_ty { &self.0 } }"
      ))
    (
      :crate-name "pleme-deref-derive"
      :description "Newtype Deref: impl Deref for Wrapper with Target = Inner."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-deref
      :kind newtype
      :spec (
        :trait-name "DerefNewtype"
        :target tuple
        :impl-template "impl ::std::ops::Deref for #self_name { type Target = #inner_ty; fn deref(&self) -> &#inner_ty { &self.0 } }"
      ))
    (
      :crate-name "pleme-inner-derive"
      :description "Newtype accessors: inner() + into_inner()."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-inner
      :kind newtype
      :spec (
        :trait-name "InnerNewtype"
        :target tuple
        :impl-template "impl #self_name { pub fn inner(&self) -> &#inner_ty { &self.0 } pub fn into_inner(self) -> #inner_ty { self.0 } }"
      ))
    (
      :crate-name "pleme-allvariants-derive"
      :description "Enum-fold: pub const ALL: &'static [Self] = &[Self::A, Self::B, ...] + pub const fn all(). Unit-variant enums only."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint enum-fold-all-variants
      :kind enum-fold
      :spec (
        :trait-name "AllVariants"
        :target unit-variants-only
        :per-variant-fragment "Self::#variant_name"
        :fold-template "impl #self_name { pub const ALL: &'static [Self] = &[#fold]; pub const fn all() -> &'static [Self] { Self::ALL } }"
      ))
    (
      :crate-name "pleme-variantcount-derive"
      :description "Enum-fold: pub const COUNT: usize = N + pub const fn count(). Any variant shape."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint enum-fold-variant-count
      :kind enum-fold
      :spec (
        :trait-name "VariantCount"
        :target any-variants
        :per-variant-fragment "1"
        :fold-template "impl #self_name { pub const COUNT: usize = #fold_count; pub const fn count() -> usize { Self::COUNT } }"
      ))
    (
      :crate-name "pleme-variantnames-derive"
      :description "Enum-fold: pub const NAMES: &'static [&'static str] = &[...] + pub const fn names(). Any variant shape."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint enum-fold-variant-names
      :kind enum-fold
      :spec (
        :trait-name "VariantNames"
        :target any-variants
        :per-variant-fragment "#variant_str"
        :fold-template "impl #self_name { pub const NAMES: &'static [&'static str] = &[#fold]; pub const fn names() -> &'static [&'static str] { Self::NAMES } }"
      ))
    (
      :crate-name "pleme-invalidating-setter-derive"
      :description "Per-field setter that bumps a cache-invalidation seqno after every assignment. Generates pub fn set_<field>(&mut self, v) { self.<field> = v; self.last_seqno = 0; } for every named field except those listed in skip_fields. Built for mado render.rs."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-field-invalidating-setter
      :kind per-field
      :spec (
        :trait-name "InvalidatingSetter"
        :target named-struct
        :per-field-template "pub fn #method_ident(&mut self, v: #field_ty) { self.#field_name = v; self.last_seqno = 0; }"
        :method-name-template "set_{}"
        :skip-fields ("last_seqno")
        :field-attribute "invalidating_setter"
      ))
    (
      :crate-name "pleme-owned-derive"
      :description "Per-field consuming getter: pub fn into_<field>(self) -> <Type>. Drops the rest of self, moves the named field out. The owned/consuming dual of GetterAll."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint per-field-owned
      :kind per-field
      :spec (
        :trait-name "OwnedAll"
        :target named-struct
        :per-field-template "pub fn #method_ident(self) -> #field_ty { self.#field_name }"
        :method-name-template "into_{}"
      ))
    (
      :crate-name "pleme-borrow-derive"
      :description "Newtype Borrow: impl ::std::borrow::Borrow<Inner> for Wrapper. Pairs with HashMap/BTreeMap lookup by inner-typed key."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-borrow
      :kind newtype
      :spec (
        :trait-name "BorrowNewtype"
        :target tuple
        :impl-template "impl ::std::borrow::Borrow<#inner_ty> for #self_name { fn borrow(&self) -> &#inner_ty { &self.0 } }"
      ))
    (
      :crate-name "pleme-borrowmut-derive"
      :description "Newtype BorrowMut: impl ::std::borrow::BorrowMut<Inner> for Wrapper. Mutable dual of BorrowNewtype."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-borrow-mut
      :kind newtype
      :spec (
        :trait-name "BorrowMutNewtype"
        :target tuple
        :impl-template "impl ::std::borrow::BorrowMut<#inner_ty> for #self_name { fn borrow_mut(&mut self) -> &mut #inner_ty { &mut self.0 } }"
      ))
    (
      :crate-name "pleme-derefmut-derive"
      :description "Newtype DerefMut: impl ::std::ops::DerefMut for Wrapper. Pairs with DerefNewtype to enable &mut Inner via auto-deref."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-deref-mut
      :kind newtype
      :spec (
        :trait-name "DerefMutNewtype"
        :target tuple
        :impl-template "impl ::std::ops::DerefMut for #self_name { fn deref_mut(&mut self) -> &mut #inner_ty { &mut self.0 } }"
      ))
    (
      :crate-name "pleme-display-derive"
      :description "Newtype Display: impl ::std::fmt::Display for Wrapper, delegating to the inner value's Display. Eliminates the boilerplate of wrapper.0.fmt(f)."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-display
      :kind newtype
      :spec (
        :trait-name "DisplayNewtype"
        :target tuple
        :impl-template "impl ::std::fmt::Display for #self_name { fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result { ::std::fmt::Display::fmt(&self.0, f) } }"
      ))
    (
      :crate-name "pleme-defaultnewtype-derive"
      :description "Newtype Default: impl ::std::default::Default for Wrapper where Inner: Default. Different from #[derive(Default)] because it doesn't require Wrapper to be PUB tuple-struct OR to implement Default itself — it forwards through the bound."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint newtype-default
      :kind newtype
      :spec (
        :trait-name "DefaultNewtype"
        :target tuple
        :impl-template "impl ::std::default::Default for #self_name where #inner_ty: ::std::default::Default { fn default() -> Self { Self(<#inner_ty as ::std::default::Default>::default()) } }"
      ))
    (
      :crate-name "pleme-variantstr-derive"
      :description "Enum-fold: pub fn as_str(&self) -> &'static str { match self { Self::A => \"A\", ... } }. Maps each unit variant to its bare name as a static string. Unit-variant enums only."
      :since "0.1.0"
      :owner "pleme-io"
      :verifier-hint enum-fold-variant-str
      :kind enum-fold
      :spec (
        :trait-name "VariantStr"
        :target unit-variants-only
        :per-variant-fragment "Self::#variant_name => #variant_str"
        :fold-template "impl #self_name { pub fn as_str(&self) -> &'static str { match self { #fold } } }"
      ))
  ))
