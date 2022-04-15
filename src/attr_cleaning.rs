use peeking_take_while::PeekableExt;
use proc_macro2::{LineColumn, Span};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, File, Ident, Meta, NestedMeta, Path};

#[derive(Debug)]
pub struct CleanError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

pub fn clean_source(source: &str) -> Result<Option<String>, CleanError> {
    let file = syn::parse_file(source).map_err(|err| {
        let span = err.span();
        let position = span.start();
        CleanError {
            line: position.line,
            column: position.column,
            message: format!("{}", err),
        }
    })?;

    let sections = get_bad_sections(&file);
    if sections.is_empty() {
        return Ok(None);
    }

    let cleaned = insert_comments(source, &sections);

    Ok(Some(cleaned))
}

fn get_bad_sections(file: &File) -> Vec<[LineColumn; 2]> {
    let mut segments = Punctuated::new();
    segments.push(Ident::new("clippy", Span::call_site()).into());
    segments.push(Ident::new("msrv", Span::call_site()).into());

    let msrv_path = Path {
        leading_colon: None,
        segments,
    };

    let mut visitor = Cleaner {
        sections: vec![],
        msrv_path,
    };

    visitor.visit_file(file);
    visitor.sections
}

struct Cleaner {
    sections: Vec<[LineColumn; 2]>,
    msrv_path: Path,
}

impl<'ast> Visit<'ast> for Cleaner {
    fn visit_attribute(&mut self, node: &'ast Attribute) {
        if let Ok(meta) = node.parse_meta() {
            let mut current = meta;
            while current.path().is_ident("cfg_attr") {
                if let Meta::List(meta_list) = current {
                    let mut nested = meta_list.nested.into_iter();
                    if let (Some(_), Some(NestedMeta::Meta(meta))) = (nested.next(), nested.next())
                    {
                        current = meta;
                    } else {
                        return;
                    }
                } else {
                    break;
                }
            }

            let path = current.path();
            if path.is_ident("allow")
                || path.is_ident("warn")
                || path.is_ident("deny")
                || *path == self.msrv_path
            {
                self.sections.push([node.span().start(), node.span().end()]);
            }
        }
    }
}

fn insert_comments(source: &str, sections: &[[LineColumn; 2]]) -> String {
    enum InsertType {
        CommentStart,
        CommentEnd,
    }

    let mut result = String::new();
    let mut inserts = sections
        .iter()
        .flat_map(|[start, end]| {
            [
                (InsertType::CommentStart, start),
                (InsertType::CommentEnd, end),
            ]
        })
        .peekable();

    let mut lines = source.lines().enumerate().peekable();

    while let Some((num, line)) = lines.next() {
        // Comment offsets are in characters
        let inserts_by_offset = inserts
            .by_ref()
            .peeking_take_while(|(_, p)| p.line == num + 1)
            .map(|(t, p)| (t, p.column))
            .scan(0, |last_col, (t, c)| {
                let result = Some((t, c - *last_col));
                *last_col = c;
                result
            });

        // Character offsets in bytes
        let mut char_indices = line.char_indices();
        let mut start = 0;
        for (insert_type, insert_offset) in inserts_by_offset {
            if let Some((char_index, c)) = char_indices.by_ref().take(insert_offset).last() {
                let end = char_index + c.len_utf8();
                result.push_str(&line[start..end]);
                start = end;
            }

            result.push_str(match insert_type {
                InsertType::CommentStart => "/* cleaned by clippy_lint_tester ",
                InsertType::CommentEnd => " */",
            });
        }

        result.push_str(&line[start..]);
        if lines.peek().is_some() {
            result.push('\n');
        }
    }
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {

    use super::clean_source;

    use expect_test::expect;
    use indoc::indoc;

    #[test]
    fn test_inner_allow() {
        let result = clean_source(indoc! {"
            #![allow(clippy::approx_constant)]
        "})
        .unwrap()
        .unwrap();

        let expected = expect![[
            r##"/* cleaned by clippy_lint_tester #![allow(clippy::approx_constant)] */"##
        ]];
        expected.assert_eq(&result);
    }

    #[test]
    fn test_inner_allow_multiline_file() {
        let result = clean_source(indoc! {"
            // Start comment

            #![allow(clippy::approx_constant)]

            fn f() { }

        "})
        .unwrap()
        .unwrap();

        let expected = expect![[r##"
            // Start comment

            /* cleaned by clippy_lint_tester #![allow(clippy::approx_constant)] */

            fn f() { }
        "##]];
        expected.assert_eq(&result);
    }

    #[test]
    fn test_inner_allow_multiline_attr() {
        let result = clean_source(indoc! {"
            // Start comment

            #![allow(
                clippy::approx_constant,
                clippy::bad_bit_mask,
            )]

            fn f() { }

        "})
        .unwrap()
        .unwrap();

        let expected = expect![[r##"
            // Start comment

            /* cleaned by clippy_lint_tester #![allow(
                clippy::approx_constant,
                clippy::bad_bit_mask,
            )] */

            fn f() { }
        "##]];
        expected.assert_eq(&result);
    }

    #[test]
    fn test_cfg_attr_feature() {
        let result = clean_source(indoc! {r##"
            #![cfg_attr(feature = "any-feature", deny(clippy, clippy_pedantic))]
        "##})
        .unwrap()
        .unwrap();

        let expected = expect![[
            r##"/* cleaned by clippy_lint_tester #![cfg_attr(feature = "any-feature", deny(clippy, clippy_pedantic))] */"##
        ]];
        expected.assert_eq(&result);
    }

    #[test]
    fn test_cfg_attr() {
        let result = clean_source(indoc! {r##"
            #![cfg_attr(any_cfg, deny(clippy::all, clippy::pedantic))]
        "##})
        .unwrap()
        .unwrap();

        let expected = expect![[
            r##"/* cleaned by clippy_lint_tester #![cfg_attr(any_cfg, deny(clippy::all, clippy::pedantic))] */"##
        ]];
        expected.assert_eq(&result);
    }

    #[test]
    fn test_cfg_attr_not_lint_setting() {
        assert!(clean_source(indoc! {r##"
                #![cfg_attr(not(target_pointer_width = "64"), path = "soft/fixslice32.rs")]
            "##})
        .unwrap()
        .is_none());
    }

    #[test]
    fn test_cfg_attr_on_tuple_field() {
        let result = clean_source(indoc! {r##"
            pub struct S(#[allow(clippy::vec_box)] RefCell<Vec<Box<u32>>>);
        "##})
        .unwrap()
        .unwrap();

        let expected = expect![[
            r##"pub struct S(/* cleaned by clippy_lint_tester #[allow(clippy::vec_box)] */ RefCell<Vec<Box<u32>>>);"##
        ]];
        expected.assert_eq(&result);
    }

    #[test]
    fn clippy_msrv_attribute() {
        let result = clean_source(indoc! {r##"
            #![feature(custom_inner_attributes)]
            #![clippy::msrv = "1.30.0"]
        "##})
        .unwrap()
        .unwrap();

        let expected = expect![[r##"
            #![feature(custom_inner_attributes)]
            /* cleaned by clippy_lint_tester #![clippy::msrv = "1.30.0"] */"##]];
        expected.assert_eq(&result);
    }
}
