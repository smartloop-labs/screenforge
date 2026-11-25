#!/usr/bin/env python3
"""The site's shared footer and promo banner: one source each, filled into every page.
Also the FAQPage structured data of any page with a <div class="faq">, built from its questions.

scripts/site/footer.en.html and footer.es.html hold the footer with links relative to the
site root. Every page under site/ gets the footer of its language (pages under site/es/ the
Spanish one), with those links rewritten relative to the page, between its <footer> tags.
scripts/gen-releases.py uses the same footer for the release notes pages.

scripts/site/banner.en.html and banner.es.html hold the promo banner (a sale, an announcement),
placed at the top of every page between <!-- banner --> and <!-- /banner -->, links rewritten
the same way. Empty or delete both files to take the banner down.

  python3 scripts/site_partials.py           # update the pages in place
  python3 scripts/site_partials.py --check   # list pages that would change; exit 1 if any

A page's FAQ structured data is the <script type="application/ld+json" data-faq> before </head>,
rebuilt from the page's <div class="faq"> (each <h3> with the text after it), so edit the questions
in the page, never the script block.

Edit the footer and the banner in scripts/site/, never in a page: the site workflow runs this before every
deploy, so a page's own copy is replaced.
"""
import html as htmllib
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
PARTIALS = ROOT / "scripts" / "site"
FOOTER = re.compile(r"<footer>.*?</footer>", re.S)
RELATIVE = re.compile(r'((?:href|src)=")(?!https?:|/|#|mailto:|data:)([^"]*)"')


def lang(page: Path) -> str:
    return "es" if page.relative_to(SITE).parts[0] == "es" else "en"


def partial(name: str, page: Path) -> str:
    """The partial `name` in the language of `page`, its links relative to the page's folder
    ("" when there is no such file)."""
    source = PARTIALS / f"{name}.{lang(page)}.html"
    text = source.read_text(encoding="utf-8").strip() if source.exists() else ""
    here = page.parent.relative_to(SITE)

    def rel(m):
        target = m.group(2)
        slash = target.endswith("/")
        out = os.path.relpath(target.rstrip("/") or ".", here).replace(os.sep, "/")
        return f'{m.group(1)}{out}{"/" if slash else ""}"'

    return RELATIVE.sub(rel, text)


def footer(page: Path) -> str:
    return partial("footer", page)


BANNER = re.compile(r"<!-- banner -->.*?<!-- /banner -->\n", re.S)


def with_banner(page: Path, html: str) -> str:
    """`html` with the banner of its language as the first thing in <body>, or with none."""
    html = BANNER.sub("", html, count=1)
    b = partial("banner", page)
    if not b:
        return html
    return html.replace("<body>\n", f"<body>\n<!-- banner -->\n{b}\n<!-- /banner -->\n", 1)


FAQ = re.compile(r'<div class="faq">(.*?)\n  </div>', re.S)
QUESTION = re.compile(r"<h3>(.*?)</h3>(.*?)(?=<h3>|$)", re.S)
FAQ_LD = re.compile(r'\n?<script type="application/ld\+json" data-faq>.*?</script>', re.S)


def text(fragment: str) -> str:
    return htmllib.unescape(re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", fragment))).strip()


def faq_ld(html: str) -> str:
    """The FAQPage block for the page's FAQ, or "" when it has none."""
    m = FAQ.search(html)
    if not m:
        return ""
    entities = [
        {"@type": "Question", "name": text(q), "acceptedAnswer": {"@type": "Answer", "text": text(a)}}
        for q, a in QUESTION.findall(m.group(1))
    ]
    data = {"@context": "https://schema.org", "@type": "FAQPage", "mainEntity": entities}
    body = json.dumps(data, ensure_ascii=False, indent=1).replace("</", "<\\/")
    return f'\n<script type="application/ld+json" data-faq>\n{body}\n</script>'


def apply(page: Path, html: str) -> str:
    html = with_banner(page, html)
    if FOOTER.search(html):
        f = footer(page)
        html = FOOTER.sub(lambda _: f, html, count=1)
    ld = faq_ld(html)
    html = FAQ_LD.sub("", html, count=1)
    if ld:
        html = html.replace("\n</head>", ld + "\n</head>", 1)
    return html


def main():
    check = "--check" in sys.argv[1:]
    changed = []
    for page in sorted(SITE.rglob("*.html")):
        html = page.read_text(encoding="utf-8")
        new = apply(page, html)
        if new != html:
            changed.append(page.relative_to(ROOT))
            if not check:
                page.write_text(new, encoding="utf-8")
    if check:
        for p in changed:
            print(f"footer, banner or FAQ data differs: {p}")
        sys.exit(1 if changed else 0)
    print(f"footer, banner and FAQ data updated on {len(changed)} page(s)")


if __name__ == "__main__":
    main()
