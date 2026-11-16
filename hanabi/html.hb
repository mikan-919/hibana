use std -> { replace }

fn escape(text: String) -> String {
    replace(replace(replace(replace(replace(text, '&', '&amp;'), '<', '&lt;'), '>', '&gt;'), '"', '&quot;'), '\'', '&#39;')
}
