/// Impl `View` trait automatically for your custom widget.`
/// ```
/// #[derive(Reactive)]
/// pub struct Home{
///     #[state]
///     list:Vec<String>
/// }
use proc_macro::TokenStream;

use proc_macro2::Span;
use quote::quote;
use syn::{
    parse, parse::Nothing, token::Colon, Error, Field, Fields, Ident, ImplItem, ImplItemFn,
    ItemImpl, ItemStruct,
};

#[proc_macro_attribute]
pub fn widget(attribute: TokenStream, item: TokenStream) -> TokenStream {
    match widget_inner(attribute, item) {
        Ok(stream) => stream,
        Err(error) => error.into_compile_error().into(),
    }
}

fn widget_inner(attribute: TokenStream, input: TokenStream) -> Result<TokenStream, Error> {
    let _: Nothing = parse(attribute)?;
    match parse(input.clone()) {
        Ok(input) => return Ok(widget_struct(input)?),
        _ => {}
    }

    widget_impl(parse(input)?)
}

fn widget_impl(mut input: ItemImpl) -> Result<TokenStream, Error> {
    for item in &mut input.items {
        match item {
            syn::ImplItem::Fn(f) => {
                let view = Ident::new("view", Span::call_site());

                let sig = f.sig.clone();
                let mut user_f = f.clone();
                user_f.sig.ident = Ident::new("__view", Span::call_site());
                let ty = *input.self_ty.clone();
                if sig.ident == view {
                    let new_f: ImplItemFn = parse(
                        quote! {fn view(&mut self) -> ::waterui_core::view::BoxView{

                            impl #ty{
                                #user_f
                            }

                            Box::new(self.__view())
                        }}
                        .into(),
                    )?;
                    *f = new_f;
                }
            }

            _ => todo!(),
        }
    }

    let frame_impl: ImplItem =
        parse(quote! {fn frame(&self) -> ::waterui_core::view::Frame{self.frame.clone()}}.into())?;
    let set_frame_impl: ImplItem = parse(
        quote! { fn set_frame(&mut self,frame: ::waterui_core::view::Frame){self.frame = frame;} }
            .into(),
    )?;

    input.items.push(frame_impl);
    input.items.push(set_frame_impl);

    Ok(quote!(#input).into())
}

fn widget_struct(mut input: ItemStruct) -> Result<TokenStream, Error> {
    let mut state_field = Vec::new();

    let struct_name = input.ident.clone();
    let mut output = TokenStream::new();

    for field in input.fields.iter_mut() {
        let state = field
            .attrs
            .iter()
            .filter(|attribute| {
                if let Some(name) = attribute.meta.path().get_ident() {
                    let state = Ident::new("state", Span::call_site());
                    *name == state
                } else {
                    false
                }
            })
            .enumerate()
            .next();
        if let Some((index, _)) = state {
            field.attrs.remove(index);
            let ty = field.ty.clone();
            field.ty = parse(quote!(::waterui_core::binding::Binding<#ty>).into())?;
            state_field.push(field.ident.clone().unwrap())
        }
    }

    let frame = Field {
        attrs: Vec::new(),
        vis: syn::Visibility::Inherited,
        mutability: syn::FieldMutability::None,
        ident: Some(Ident::new("frame", Span::call_site())),
        colon_token: Some(Colon::default()),
        ty: parse(quote!(::waterui_core::view::Frame).into())?,
    };

    let fields;

    match &mut input.fields {
        Fields::Named(named_fields) => fields = named_fields,
        _ => unreachable!(),
    }

    fields.named.push(frame);

    output.extend::<TokenStream>(
        quote! {
            #input
            impl ::waterui_core::view::Reactive for #struct_name{
                fn is_reactive(&self) -> bool {
                    true
                }

                fn subscribe(&self, subscriber: fn() -> ::waterui_core::binding::BoxSubscriber) {
                    #(self.#state_field.add_boxed_subscriber((subscriber)()));*
                }
            }
        }
        .into(),
    );

    Ok(output)
}
