"""Writes site/llms-full.txt: the text of the site's main pages in one Markdown file, for AI
tools that read a whole site at once (llms.txt is the short index, this is the full text).

The pages' own copy is the source, so run this after editing any of PAGES:

    python3 scripts/gen-llms-full.py
"""
import pathlib
import re
from html.parser import HTMLParser

ROOT = pathlib.Path(__file__).resolve().parents[1] / 'site'
BASE = 'https://screenforge.studio/'
# file, the page's address, its heading here
PAGES = [
    ('index.html', '', 'Home'),
    ('download.html', 'download', 'Download'),
    ('pricing.html', 'pricing', 'Pricing'),
    ('comparison.html', 'comparison', 'Comparison: Jam vs Screen Studio vs ScreenForge'),
    ('agents.html', 'agents', 'AI agents (MCP server)'),
    ('privacy.html', 'privacy', 'Privacy policy'),
    ('terms.html', 'terms', 'Terms'),
]

# never part of the copy: chrome, code for the browser, controls, things read as pictures
SKIP = {'script', 'style', 'svg', 'nav', 'footer', 'noscript', 'form', 'button', 'video', 'template', 'head', 'select'}
BLOCK = {'p', 'div', 'section', 'ul', 'ol', 'table', 'pre', 'blockquote', 'header', 'figure', 'figcaption', 'details', 'summary', 'dl', 'dt', 'dd'}
HEADINGS = {'h1': '##', 'h2': '##', 'h3': '###', 'h4': '####'}
VOID = {'br', 'img', 'hr', 'input', 'meta', 'link', 'source', 'track', 'wbr', 'col'}


class Text(HTMLParser):
    def __init__(self, main_only):
        super().__init__(convert_charrefs=True)
        self.out = []          # finished lines
        self.buf = []          # the line being read
        self.skip = 0          # depth inside a SKIP element
        self.hidden = []       # depth markers of aria-hidden elements
        self.depth = 0
        self.inside = not main_only
        self.main_only = main_only
        self.prefix = ''       # '- ' in a list item, '## ' in a heading
        self.row = None        # cells of the table row being read
        self.label = None      # an h1's aria-label, used in place of its animated words
        self.pre = 0

    def flush(self):
        text = ''.join(self.buf)
        self.buf = []
        if not self.pre:
            text = re.sub(r'\s+', ' ', text).strip()
        if text:
            self.out.append(self.prefix + text)
            self.out.append('')

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag not in VOID:
            self.depth += 1
        if tag == 'main' and self.main_only:
            self.inside = True
        if self.skip or tag in SKIP:
            if tag not in VOID:
                self.skip += 1
            return
        if a.get('aria-hidden') == 'true' and tag not in VOID:
            self.hidden.append(self.depth)
            return
        if self.hidden or not self.inside:
            return
        if self.row is not None and tag not in ('td', 'th'):
            # inside a table cell everything stays on the row; parts are set apart by a space
            self.buf.append(' ')
            return
        if tag in HEADINGS:
            self.flush()
            self.prefix = HEADINGS[tag] + ' '
            self.label = a.get('aria-label')
        elif tag == 'li':
            self.flush()
            self.prefix = '- '
        elif tag == 'tr':
            self.flush()
            self.row = []
        elif tag in ('td', 'th'):
            self.buf = []
        elif tag == 'br':
            self.buf.append(' ')
        elif tag == 'pre':
            self.flush()
            self.out.append('```')
            self.pre += 1
        elif tag == 'code' and not self.pre:
            self.buf.append('`')
        elif tag in BLOCK:
            self.flush()

    def handle_endtag(self, tag):
        if tag in VOID:
            return
        depth, self.depth = self.depth, self.depth - 1
        if self.skip:
            self.skip -= 1
            return
        if self.hidden:
            if self.hidden[-1] == depth:
                self.hidden.pop()
            return
        if not self.inside:
            return
        if tag == 'main' and self.main_only:
            self.flush()
            self.inside = False
        elif tag in HEADINGS:
            if self.label:
                self.buf = [self.label]
            self.flush()
            self.prefix, self.label = '', None
        elif tag == 'li':
            self.flush()
            self.prefix = ''
            # list items sit on consecutive lines
            if self.out and self.out[-1] == '':
                self.out.pop()
        elif tag in ('td', 'th') and self.row is not None:
            self.row.append(re.sub(r'\s+', ' ', ''.join(self.buf)).strip().replace('|', '/'))
            self.buf = []
        elif tag == 'tr' and self.row is not None:
            if any(self.row):
                self.out.append('| ' + ' | '.join(self.row) + ' |')
            self.row = None
        elif tag == 'table':
            self.out.append('')
        elif tag == 'pre':
            self.pre -= 1
            text = ''.join(self.buf).strip('\n')
            self.buf = []
            self.out.extend([text, '```', ''])
        elif tag == 'code' and not self.pre:
            self.buf.append('`')
        elif tag in ('ul', 'ol'):
            self.flush()
            self.out.append('')
        elif tag in BLOCK:
            self.flush()

    def handle_data(self, data):
        if self.skip or self.hidden or not self.inside:
            return
        self.buf.append(data)


def page_text(file):
    html = (ROOT / file).read_text()
    parser = Text(main_only='<main' in html)
    parser.feed(html)
    parser.flush()
    lines = []
    for line in parser.out:
        # one blank line between blocks
        if line == '' and (not lines or lines[-1] == ''):
            continue
        lines.append(line)
    return '\n'.join(lines).strip()


def main():
    index = (ROOT / 'llms.txt').read_text()
    # the title and summary of llms.txt open this file too
    head = index.split('\n## ', 1)[0].strip()
    parts = [head, '', 'This is the full text of the main pages of screenforge.studio. The short index is at ' + BASE + 'llms.txt.', '']
    for file, path, title in PAGES:
        parts += ['---', '', f'# {title}', '', f'Source: {BASE}{path}', '', page_text(file), '']
    (ROOT / 'llms-full.txt').write_text('\n'.join(parts).rstrip() + '\n')


if __name__ == '__main__':
    main()
