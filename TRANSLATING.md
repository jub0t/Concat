# Translating Concat

The interface is being rebuilt in Slint, and its strings are not yet
wrapped for translation. The earlier web-based editor shipped with a
JSON-per-language scheme and a Simplified Chinese translation; that scheme
went with it.

The plan is Slint's own `@tr()` mechanism, which produces standard gettext
`.po` files translators already know how to work with. Until that lands
there is nothing to translate yet - watch this file and the changelog.
