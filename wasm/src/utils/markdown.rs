use pulldown_cmark::{Options, Parser, html};

pub(crate) fn to_html(md: &str) -> String {
    let mut opts = Options::empty();

    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);

    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, opts));

    out
}
