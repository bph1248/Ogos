use proc_macro2::*;
use quote::*;
use std::collections::{HashMap, HashSet};
use syn::{
    parse::*,
    *
};

struct ChangelingRels(Vec<TokenStream>);
impl ToTokens for ChangelingRels {
    fn to_tokens(&self, token_stream: &mut TokenStream) {
        token_stream.append_all(&self.0);
    }
}

struct LocVarIdents(HashSet<Ident>);
impl ToTokens for LocVarIdents {
    fn to_tokens(&self, token_stream: &mut TokenStream) {
        token_stream.append_separated(&self.0, Punct::new(',', Spacing::Alone));
    }
}

struct LocalRels {
    err_loc_ty: Type,
    loc_var_ty: Type,
    loc_var_changeling: Ident,
    loc_var_idents: LocVarIdents,
    rels: HashMap<Ident, Vec<Ident>>
}
impl Parse for LocalRels {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut loc_var_idents = LocVarIdents(HashSet::<Ident>::new());
        let mut rels = HashMap::<Ident, Vec<Ident>>::new();

        let err_loc_ty = input.parse::<Type>()?;
        input.parse::<Token![,]>()?;

        let loc_var_ty = input.parse::<Type>()?;
        input.parse::<Token![,]>()?;

        let loc_var_changeling = input.parse::<Ident>()?;
        input.parse::<Token![,]>()?;

        while let Ok(into) = input.parse::<Ident>() {
            loc_var_idents.0.insert(into.clone()); // Catch all intos as enum vars

            let froms = input.parse::<Token![=]>()
                .and_then(|_| {
                    let braced;
                    _ = braced!(braced in input);

                    let froms = braced.parse_terminated(Ident::parse, Token![,])?;
                    let froms = froms.into_iter().collect::<Vec<_>>();

                    for from in froms.iter() {
                        loc_var_idents.0.insert(from.clone()); // Catch all froms as enum vars
                    }

                    Ok(froms)
                })
                .unwrap_or_default();

            rels.insert(into, froms);

            if input.peek(Token![,]) {
                _ = input.parse::<Token![,]>();
            }
        }

        Ok(
            LocalRels {
                err_loc_ty,
                loc_var_ty,
                loc_var_changeling,
                loc_var_idents,
                rels
            }
        )
    }
}

struct TransRels {
    err_loc_ty: Type,
    loc_var_ty: Type,
    rels: HashMap<Ident, HashSet<Ident>>
}
impl ToTokens for TransRels {
    fn to_tokens(&self, token_stream: &mut TokenStream) {
        let mut ts = Vec::new();

        for (into, froms) in self.rels.iter() {
            for from in froms.iter() {
                let TransRels {
                    err_loc_ty,
                    loc_var_ty,
                    ..
                } = &self;
                let from_body = from_body();

                ts.push(
                    quote! {
                        impl From<#err_loc_ty<{ #loc_var_ty::#from as u32 }>> for #err_loc_ty<{ #loc_var_ty::#into as u32 }> {
                            #[track_caller]
                            fn from(value: #err_loc_ty<{ #loc_var_ty::#from as u32 }>) -> Self {
                                #from_body
                            }
                        }
                    }
                );
            }
        }

        token_stream.append_all(ts);
    }
}

fn from_body() -> TokenStream {
    quote! {
        Self {
            var: value.var,
            msg: value.msg,
            trail: {
                let loc = Loc {
                    file: panic::Location::caller().file(),
                    line: panic::Location::caller().line()
                };

                match value.trail {
                    Some(mut trail) => {
                        trail.push(loc);

                        Some(trail)
                    },
                    None => Some(vec![loc])
                }
            },
            x: value.x
        }
    }
}

fn rec(intos: &mut Vec<Ident>, froms: &[Ident], local_rels: &HashMap<Ident, Vec<Ident>>, trans_rels: &mut HashMap::<Ident, HashSet<Ident>>) {
    for from in froms.iter() {
        for into in intos.iter() { // Form rels for all intos that have come higher up
            trans_rels.entry(into.clone())
                .and_modify(|froms| {
                    froms.insert(from.clone());
                })
                .or_insert(HashSet::<Ident>::from([from.clone()]));
        }

        let deeper_froms = local_rels.get(from).unwrap();
        if !deeper_froms.is_empty() {
            intos.push(from.clone());
            rec(intos, deeper_froms, local_rels, trans_rels);
            intos.pop();
        }
    }
}

#[proc_macro]
pub fn err_loc_sets(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let LocalRels {
        err_loc_ty,
        loc_var_ty,
        loc_var_changeling,
        loc_var_idents,
        rels: local_rels,
    } = parse_macro_input!(input as LocalRels);

    // Build From impls from local rels
    let mut trans_rels = TransRels {
        err_loc_ty: err_loc_ty.clone(),
        loc_var_ty: loc_var_ty.clone(),
        rels: HashMap::<Ident, HashSet<Ident>>::new(),
    };
    for (into, froms) in local_rels.iter() {
        let mut intos = vec![into.clone()];
        rec(&mut intos, froms, &local_rels, &mut trans_rels.rels);
    }

    // Quote From impls for changeling
    let changeling_rels = ChangelingRels(
        {
            let from_body = from_body();

            loc_var_idents.0.iter()
                .map(|loc_var_ident| {
                    quote! {
                        impl From<#err_loc_ty<{ #loc_var_ty::#loc_var_ident as u32 }>> for #err_loc_ty<{ #loc_var_ty::#loc_var_changeling as u32 }> {
                            #[track_caller]
                            fn from(value: #err_loc_ty<{ #loc_var_ty::#loc_var_ident as u32 }>) -> Self {
                                #from_body
                            }
                        }
                        impl From<#err_loc_ty<{ #loc_var_ty::#loc_var_changeling as u32 }>> for #err_loc_ty<{ #loc_var_ty::#loc_var_ident as u32 }> {
                            #[track_caller]
                            fn from(value: #err_loc_ty<{ #loc_var_ty::#loc_var_changeling as u32 }>) -> Self {
                                #from_body
                            }
                        }
                    }
                })
                .collect::<Vec<_>>()
        }
    );

    quote! {
        // stringify! {
            #[repr(u32)]
            pub(crate) enum #loc_var_ty {
                #loc_var_changeling,
                #loc_var_idents
            }

            #changeling_rels
            #trans_rels
        // }
    }
    .into()
}
