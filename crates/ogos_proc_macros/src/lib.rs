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
            pub enum #loc_var_ty {
                #loc_var_changeling,
                #loc_var_idents
            }

            #changeling_rels
            #trans_rels
        // }
    }
    .into()
}

const MACRO_SUPPORT_REGULAR_STRUCTS: &str = "Struct must be regular";
const MACRO_SUPPORT_ANNOTATION_VALUE_FN_IDENT: &str = "Annotation value must be usable as a fn identifier";
const MACRO_SUPPORT_ANNOTATION_FORM_NAME_VALUE: &str = "Annotation must be of the form 'field_visit = \"value\"'";

#[proc_macro_derive(FieldIter, attributes(field_iter))]
pub fn field_iter(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let struct_ident = &input.ident;
    let elided_lifetimes = input.generics.lifetimes().next().map(|_| quote! { <'_> });

    let mut annotated_fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => fields.named.iter()
                .filter(|field|
                    field.attrs.iter().any(|attr| attr.path().get_ident().is_some_and(|ident| ident == "field_iter")))
                .peekable(),
            _ => panic!("{MACRO_SUPPORT_REGULAR_STRUCTS}")
        },
        _ => panic!("{MACRO_SUPPORT_REGULAR_STRUCTS}")
    };
    let (
        iter_item_ty,
        iterable_field_idents
    ) : (
        _,
        [Box<dyn Iterator<Item = _>>; _]
    ) = match annotated_fields.peek() {
        Some(_) => {
            let ty = &annotated_fields.peek().unwrap().ty;
            let iterable_field_idents_1 = Box::new(annotated_fields.map(|field| &field.ident));
            let iterable_field_idents_2 = Box::new(iterable_field_idents_1.as_ref().clone());

            (ty, [iterable_field_idents_1, iterable_field_idents_2])
        },
        None => match &input.data {
            Data::Struct(data_struct) => match &data_struct.fields {
                Fields::Named(fields) => {
                    let ty = &fields.named[0].ty;
                    let iterable_field_idents_1 = Box::new(fields.named.iter().map(|field| &field.ident));
                    let iterable_field_idents_2 = Box::new(iterable_field_idents_1.as_ref().clone());

                    (ty, [iterable_field_idents_1, iterable_field_idents_2])
                },
                _ => panic!("{MACRO_SUPPORT_REGULAR_STRUCTS}")
            },
            _ => panic!("{MACRO_SUPPORT_REGULAR_STRUCTS}")
        }
    };
    let [
        iterable_field_idents_1,
        iterable_field_idents_2
    ] = iterable_field_idents;

    quote! {
        impl #struct_ident #elided_lifetimes {
            pub fn iter(&self) -> impl Iterator<Item = &#iter_item_ty> {
                [
                    #(&self.#iterable_field_idents_1),*
                ]
                .into_iter()
            }

            pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut #iter_item_ty> {
                [
                    #(&mut self.#iterable_field_idents_2),*
                ]
                .into_iter()
            }
        }
    }
    .into()
}

#[proc_macro_derive(FieldVisit, attributes(field_visit))]
pub fn field_visit(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let struct_ident = &input.ident;
    let elided_lifetimes = input.generics.lifetimes().next().map(|_| quote! { <'_> });

    let mut annotated_field_idents: HashMap<Ident, (Vec<&Ident>, HashSet<&Type>)> = HashMap::new();

    match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => for field in &fields.named {
                if let Some(attr) = field.attrs.iter().find(|attr| attr.path().get_ident().is_some_and(|ident| ident == "field_visit")) {
                    if let Meta::NameValue(nv) = &attr.meta &&
                        let Expr::Lit(ExprLit { lit, .. }) = &nv.value &&
                        let Lit::Str(lit_str) = lit
                    {
                        let method_name = lit_str.value();
                        let method_ident = parse_str::<Ident>(&method_name).expect(MACRO_SUPPORT_ANNOTATION_VALUE_FN_IDENT);
                        let field_ident = field.ident.as_ref().unwrap();
                        let field_ty = &field.ty;

                        annotated_field_idents.entry(method_ident)
                            .and_modify(|(field_idents, field_tys)| {
                                field_idents.push(field_ident);
                                field_tys.insert(field_ty);
                            })
                            .or_insert((
                                vec![field_ident],
                                HashSet::from([field_ty])
                            ));
                    } else {
                        panic!("{MACRO_SUPPORT_ANNOTATION_FORM_NAME_VALUE}")
                    }
                }
            },
            _ => panic!("{MACRO_SUPPORT_REGULAR_STRUCTS}")
        },
        _ => panic!("{MACRO_SUPPORT_REGULAR_STRUCTS}")
    }

    let method_impls_iter = annotated_field_idents.into_iter()
        .map(|(method_name, (field_idents, field_tys))| {
            let field_tys = field_tys.iter();

            quote! {
                fn #method_name<V>(&mut self, mut visitor: V) where
                    V: #(FieldVisitor<#field_tys>)+*
                {
                    #(visitor.visit(&mut self.#field_idents);)*
                }
            }
        });

    quote! {
        impl #struct_ident #elided_lifetimes {
            #(#method_impls_iter)*
        }
    }
    .into()
}
