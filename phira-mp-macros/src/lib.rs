use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{
    parse_macro_input, Data, DataEnum, DataStruct, DeriveInput, Fields, GenericArgument,
    PathArguments, Type, Variant,
};

#[proc_macro_derive(BinaryData)]
pub fn derive_model_ex(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let res = build_derive(input.ident, input.data);
    quote! {
        #res
    }
    .into()
}

struct TypeInfo {
    is_arc: bool,
    is_vec: bool,
}

fn parse_type(typ: &Type) -> TypeInfo {
    let (typ, is_arc) = match typ {
        Type::Path(path) => {
            let last = path.path.segments.last().unwrap();
            if last.ident == "Arc" {
                (
                    match &last.arguments {
                        PathArguments::AngleBracketed(arg) => match &arg.args[0] {
                            GenericArgument::Type(typ) => typ,
                            _ => unreachable!(),
                        },
                        _ => unreachable!(),
                    },
                    true,
                )
            } else {
                (typ, false)
            }
        }
        _ => (typ, false),
    };
    let (_typ, is_vec) = match typ {
        Type::Path(ref path) => {
            let last = path.path.segments.last().unwrap();
            if last.ident == "Vec" {
                (
                    match &last.arguments {
                        PathArguments::AngleBracketed(arg) => match &arg.args[0] {
                            GenericArgument::Type(typ) => typ,
                            _ => unreachable!(),
                        },
                        _ => unreachable!(),
                    },
                    true,
                )
            } else {
                (typ, false)
            }
        }
        _ => (typ, false),
    };
    TypeInfo { is_arc, is_vec }
}

fn build_derive(name: Ident, data: Data) -> TokenStream {
    match data {
        Data::Struct(DataStruct { fields, .. }) => build_derive_struct(name, fields),
        Data::Enum(DataEnum { variants, .. }) => {
            build_derive_enum(name, variants.into_iter().collect())
        }
        _ => panic!(),
    }
}

fn build_derive_struct(name: Ident, fields: Fields) -> TokenStream {
    let fields: Vec<_> = fields
        .iter()
        .map(|it| (it.ident.clone(), parse_type(&it.ty)))
        .collect();
    let read = struct_read(&fields);
    let write = struct_write(&fields, true);
    quote! {
        impl crate::BinaryData for #name {
            fn read_binary(r: &mut crate::BinaryReader<'_>) -> Result<Self> {
                Ok(Self { #read })
            }

            fn write_binary(&self, w: &mut crate::BinaryWriter<'_>) -> Result<()> {
                #write
                Ok(())
            }
        }
    }
}

fn build_derive_enum(name: Ident, variants: Vec<Variant>) -> TokenStream {
    let read_arms = variants
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let i = i as u8;
            let name = &it.ident;
            match &it.fields {
                Fields::Unit => quote! { #i => Self::#name },
                Fields::Unnamed(fields) => {
                    let fields = struct_read(
                        &fields
                            .unnamed
                            .iter()
                            .map(|it| (None, parse_type(&it.ty)))
                            .collect::<Vec<_>>(),
                    );
                    quote! { #i => Self::#name(#fields) }
                }
                Fields::Named(fields) => {
                    let fields = struct_read(
                        &fields
                            .named
                            .iter()
                            .map(|it| (it.ident.clone(), parse_type(&it.ty)))
                            .collect::<Vec<_>>(),
                    );
                    quote! { #i => Self::#name { #fields } }
                }
            }
        })
        .chain(std::iter::once(
            quote! { x => anyhow::bail!("invalid enum: {}", x) },
        ));
    let write_arms = variants.iter().enumerate().map(|(i, it)| {
        let i = i as u8;
        let name = &it.ident;
        match &it.fields {
            Fields::Unit => quote! { Self::#name => w.write_val(#i)? },
            Fields::Unnamed(fields) => {
                let names: Vec<_> = (0..fields.unnamed.len())
                    .map(|i| {
                        let ident = Ident::new(&format!("_{i}"), Span::call_site());
                        quote! { #ident }
                    })
                    .collect();
                let writes = struct_write(
                    &fields
                        .unnamed
                        .iter()
                        .map(|it| (None, parse_type(&it.ty)))
                        .collect::<Vec<_>>(),
                    false,
                );
                quote! { Self::#name(#(#names,)*) => { w.write_val(#i)?; #writes } }
            }
            Fields::Named(fields) => {
                let names: Vec<_> = fields
                    .named
                    .iter()
                    .map(|it| {
                        let ident = it.ident.clone().unwrap();
                        quote! { #ident }
                    })
                    .collect();
                let writes = struct_write(
                    &fields
                        .named
                        .iter()
                        .map(|it| (it.ident.clone(), parse_type(&it.ty)))
                        .collect::<Vec<_>>(),
                    false,
                );
                quote! { Self::#name { #(#names,)* } => { w.write_val(#i)?; #writes } }
            }
        }
    });
    quote! {
        impl crate::BinaryData for #name {
            fn read_binary(r: &mut crate::BinaryReader<'_>) -> Result<Self> {
                Ok(match r.read::<u8>()? {
                    #(#read_arms,)*
                })
            }

            fn write_binary(&self, w: &mut crate::BinaryWriter<'_>) -> Result<()> {
                match self {
                    #(#write_arms,)*
                }
                Ok(())
            }
        }
    }
}

fn struct_read(fields: &[(Option<Ident>, TypeInfo)]) -> TokenStream {
    let fields = fields.iter().map(|(name, typ)| field_read(name, typ));
    quote! { #(#fields,)* }
}

fn field_read(name: &Option<Ident>, typ: &TypeInfo) -> TokenStream {
    let val = match (typ.is_arc, typ.is_vec) {
        (false, false) => quote! { r.read()? },
        (false, true) => quote! { r.array()? },
        (true, false) => quote! { r.read()?.into() },
        (true, true) => quote! { r.array()?.into() },
    };
    if let Some(name) = name {
        quote! { #name: #val }
    } else {
        val
    }
}

fn struct_write(fields: &[(Option<Ident>, TypeInfo)], use_self: bool) -> TokenStream {
    fields
        .iter()
        .enumerate()
        .map(|(i, (name, typ))| {
            field_write(
                if use_self {
                    let i = i as u8;
                    if let Some(name) = name {
                        quote! { &self.#name }
                    } else {
                        quote! { &self.#i }
                    }
                } else {
                    let name = name
                        .clone()
                        .unwrap_or(Ident::new(&format!("_{i}"), Span::call_site()));
                    quote! { #name }
                },
                typ,
            )
        })
        .collect()
}

fn field_write(field: TokenStream, typ: &TypeInfo) -> TokenStream {
    if typ.is_vec {
        quote! { w.array(#field)?; }
    } else {
        quote! { w.write(#field)?; }
    }
}
