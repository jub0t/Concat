# Concat licensing exceptions

Concat is licensed under the **GNU Affero General Public License, version 3 or
later** (AGPL-3.0-or-later). The full text is in [`LICENSE`](LICENSE).

The additional permission below is granted by the copyright holders on top of
that license. It grants rights; it never takes any away. If you ignore this
file entirely, plain AGPL-3.0-or-later applies and you are still compliant.

---

## The Concat Plugin Exception, version 1.0

### Why this exists

The **Concat API** is a stable interface that lets plugins, scripts, automation
and MCP integrations extend the editor without forking it.

Without this exception, a plugin that links against the Concat API would form a
combined work with Concat, and the AGPL would reach the plugin's own source.
That is not the intent. The copyleft is here to keep *Concat itself* — the
engine, the compositor, the renderer, the editor — open. It is not here to
dictate the license of software that merely talks to it.

### The grant

As a special exception, the copyright holders of Concat give you permission to
combine an Independent Module with Concat to produce a combined work, and to
convey the resulting combined work, **without the Independent Module itself
becoming subject to the requirements of the AGPL**, under the conditions below.

An **Independent Module** is a module that:

1. is not derived from, and does not incorporate any part of, Concat's source
   code; and
2. interfaces with Concat exclusively through the Concat API, its plugin
   entry points, its command/IPC surface, or its documented file and project
   formats.

### Conditions

This exception applies only while all of the following hold.

1. **Concat stays AGPL.** Every portion of Concat in the combined work — the
   original source and any modification you make to it — remains licensed
   under AGPL-3.0-or-later, and you meet the AGPL's obligations for it,
   including the source-offer requirement of section 13 when the combined work
   is made available to users over a network.
2. **The API is the boundary.** The Independent Module reaches Concat only
   through the interfaces named above. Statically or dynamically linking
   against Concat's internal crates, vendoring its source, or reproducing a
   substantial portion of its code in your module places that module outside
   this exception.
3. **No relabelling.** This exception does not permit conveying Concat, a
   modified Concat, or a substantial portion of Concat under terms other than
   the AGPL by describing it as a plugin, a host, a runtime, or a bundled
   dependency of an Independent Module.
4. **Trademarks are separate.** This exception grants no rights in the Concat
   name or logo. See [`TRADEMARK.md`](TRADEMARK.md).

### Notes

If you modify Concat, you may extend this exception to your version, but you
are not obliged to. If you do not wish to do so, delete this file from your
version.

Nothing here limits the permissions the AGPL already grants you, and nothing
here is a licence to any patent beyond what AGPL-3.0-or-later section 11
already provides.

---

## Commercial licensing

The AGPL's reciprocity is deliberate: if you build on Concat's engine, the
result stays open for the people who use it.

If that does not fit your situation, Concat's copyright is held centrally (all
contributions are made under the CLA in [`CLA.md`](CLA.md)), so a separate
commercial licence can be granted without the AGPL's obligations. Enquiries:
**jub0trd@gmail.com**.
