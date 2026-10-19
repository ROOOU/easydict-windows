/**
 * A deliberately small Markdown renderer.
 *
 * LLM services often answer with Markdown; the macOS app hands that to
 * `AttributedString`/`MarkdownUI`. There is no such dependency here and none is
 * allowed, so this module parses the subset the services actually emit —
 * headings, bold, italics, inline code, fenced code, lists, quotes, rules and
 * links — and emits React elements.
 *
 * Nothing here ever produces markup: every fragment becomes either a React text
 * node (which React escapes) or an element this file creates. That is what lets
 * `dangerouslySetInnerHTML` stay out of the codebase even though the input is
 * model output.
 */

import { Fragment, type ReactNode } from "react";

import * as api from "../api";

export interface MarkdownProps {
  text: string;
  /** Rendered as a `mark` behind each occurrence of the query. Skipped inside code. */
  highlight?: string | null;
}

type Block =
  | { kind: "code"; language: string; code: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "rule" }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; lines: string[] }
  | { kind: "paragraph"; text: string };

const FENCE = /^\s*(```|~~~)\s*([\w+#.-]*)\s*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const RULE = /^\s{0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const ORDERED = /^\s*\d{1,9}[.)]\s+(.*)$/;

/** `**bold**`, `*italic*`, `` `code` ``, `~~strike~~`, `[label](url)`, `![alt](url)`. */
const INLINE =
  /(!\[[^\]\n]*\]\([^)\s]*\)|\*\*[^*\n]+\*\*|\*[^*\n]+\*|~~[^~\n]+~~|`[^`\n]+`|\[[^\]\n]*\]\([^)\s]+\))/g;
const ESCAPES = /\\([\\`*_[\]()#+\-.!~>])/g;

function unescapeText(text: string): string {
  return text.replace(ESCAPES, "$1");
}

/**
 * Only schemes that make sense to hand to the OS browser. `javascript:`,
 * `data:` and friends fall through to plain text.
 */
function safeUrl(url: string): string | null {
  const trimmed = url.trim();
  if (/^https?:\/\//i.test(trimmed) || /^mailto:/i.test(trimmed)) {
    return trimmed;
  }
  return null;
}

function parseBlocks(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraph: string[] = [];
  let index = 0;

  const flushParagraph = () => {
    if (paragraph.length > 0) {
      blocks.push({ kind: "paragraph", text: paragraph.join("\n") });
      paragraph = [];
    }
  };

  while (index < lines.length) {
    const line = lines[index];

    const fence = FENCE.exec(line);
    if (fence) {
      flushParagraph();
      const marker = fence[1];
      const body: string[] = [];
      index += 1;
      while (index < lines.length && !lines[index].trim().startsWith(marker)) {
        body.push(lines[index]);
        index += 1;
      }
      // Skip the closing fence; an unclosed fence just ends at the last line.
      index += 1;
      blocks.push({ kind: "code", language: fence[2] ?? "", code: body.join("\n") });
      continue;
    }

    if (line.trim().length === 0) {
      flushParagraph();
      index += 1;
      continue;
    }

    const heading = HEADING.exec(line);
    if (heading) {
      flushParagraph();
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2] });
      index += 1;
      continue;
    }

    if (RULE.test(line)) {
      flushParagraph();
      blocks.push({ kind: "rule" });
      index += 1;
      continue;
    }

    if (QUOTE.test(line)) {
      flushParagraph();
      const body: string[] = [];
      while (index < lines.length) {
        const quoted = QUOTE.exec(lines[index]);
        if (!quoted) {
          break;
        }
        body.push(quoted[1]);
        index += 1;
      }
      blocks.push({ kind: "quote", lines: body });
      continue;
    }

    const ordered = ORDERED.test(line);
    if (ordered || BULLET.test(line)) {
      flushParagraph();
      const items: string[] = [];
      while (index < lines.length) {
        const current = lines[index];
        const match = ordered ? ORDERED.exec(current) : BULLET.exec(current);
        if (match) {
          items.push(match[1]);
          index += 1;
          continue;
        }
        // An indented line continues the item above it.
        if (/^\s{2,}\S/.test(current) && items.length > 0) {
          items[items.length - 1] = `${items[items.length - 1]}\n${current.trim()}`;
          index += 1;
          continue;
        }
        break;
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }

    paragraph.push(line);
    index += 1;
  }

  flushParagraph();
  return blocks;
}

/** Splits one line into text runs and the elements the inline syntax asks for. */
function renderInline(text: string, highlight: string | null | undefined): ReactNode[] {
  const nodes: ReactNode[] = [];
  let cursor = 0;
  let key = 0;
  // `replace` with a zero-width guard lets us walk the matches in order while
  // keeping the plain runs between them as text nodes.
  const pattern = new RegExp(INLINE.source, "g");
  for (;;) {
    const match = pattern.exec(text);
    if (!match) {
      break;
    }
    const [token] = match;
    if (match.index > cursor) {
      nodes.push(
        highlightRun(unescapeText(text.slice(cursor, match.index)), highlight, key),
      );
      key += 1;
    }
    nodes.push(renderToken(token, key, highlight));
    key += 1;
    cursor = match.index + token.length;
  }
  if (cursor < text.length) {
    nodes.push(highlightRun(unescapeText(text.slice(cursor)), highlight, key));
  }
  return nodes;
}

function highlightRun(
  text: string,
  highlight: string | null | undefined,
  key: number,
): ReactNode {
  const needle = highlight?.trim() ?? "";
  if (!needle || needle.length > 40) {
    return text;
  }
  const lowerText = text.toLocaleLowerCase();
  const lowerNeedle = needle.toLocaleLowerCase();
  const parts: ReactNode[] = [];
  let cursor = 0;
  for (;;) {
    const found = lowerText.indexOf(lowerNeedle, cursor);
    if (found === -1) {
      break;
    }
    if (found > cursor) {
      parts.push(text.slice(cursor, found));
    }
    parts.push(
      <mark className="ez-mark" key={`mark-${found}`}>
        {text.slice(found, found + needle.length)}
      </mark>,
    );
    cursor = found + needle.length;
  }
  if (parts.length === 0) {
    return text;
  }
  if (cursor < text.length) {
    parts.push(text.slice(cursor));
  }
  return <Fragment key={`run-${key}`}>{parts}</Fragment>;
}

function renderToken(
  token: string,
  key: number,
  highlight: string | null | undefined,
): ReactNode {
  const image = /^!\[([^\]\n]*)\]\(([^)\s]*)\)$/.exec(token);
  if (image) {
    // Remote images are not fetched: the alt text is all that is shown.
    return (
      <span className="ez-md-image" key={`image-${key}`}>
        {unescapeText(image[1])}
      </span>
    );
  }

  const link = /^\[([^\]\n]*)\]\(([^)\s]+)\)$/.exec(token);
  if (link) {
    const url = safeUrl(link[2]);
    const label = renderInline(link[1], null);
    if (!url) {
      return <Fragment key={`link-${key}`}>{label}</Fragment>;
    }
    return (
      <a
        className="ez-md-link"
        key={`link-${key}`}
        href={url}
        onClick={(event) => {
          event.preventDefault();
          void api.openLink(url);
        }}
      >
        {label}
      </a>
    );
  }

  if (token.startsWith("**") || token.startsWith("~~")) {
    const strong = token.startsWith("**");
    const inner = token.slice(2, -2);
    return strong ? (
      <strong key={`strong-${key}`}>{renderInline(inner, highlight)}</strong>
    ) : (
      <del key={`del-${key}`}>{renderInline(inner, highlight)}</del>
    );
  }

  if (token.startsWith("`")) {
    return (
      <code className="ez-md-code" key={`code-${key}`}>
        {token.slice(1, -1)}
      </code>
    );
  }

  return <em key={`em-${key}`}>{renderInline(token.slice(1, -1), highlight)}</em>;
}

function renderBlock(block: Block, index: number, highlight: string | null | undefined) {
  switch (block.kind) {
    case "code":
      return (
        <pre className="ez-md-pre" key={`block-${index}`}>
          {block.language ? (
            <span className="ez-md-lang">{block.language}</span>
          ) : null}
          <code>{block.code}</code>
        </pre>
      );
    case "heading": {
      const level = Math.min(Math.max(block.level, 1), 6);
      const Tag = `h${level}` as "h1" | "h2" | "h3" | "h4" | "h5" | "h6";
      return (
        <Tag className={`ez-md-h ez-md-h${level}`} key={`block-${index}`}>
          {renderInline(block.text, highlight)}
        </Tag>
      );
    }
    case "rule":
      return <hr className="ez-md-hr" key={`block-${index}`} />;
    case "list":
      return block.ordered ? (
        <ol className="ez-md-list" key={`block-${index}`}>
          {block.items.map((item, itemIndex) => (
            <li key={`item-${itemIndex}`}>{renderInline(item, highlight)}</li>
          ))}
        </ol>
      ) : (
        <ul className="ez-md-list" key={`block-${index}`}>
          {block.items.map((item, itemIndex) => (
            <li key={`item-${itemIndex}`}>{renderInline(item, highlight)}</li>
          ))}
        </ul>
      );
    case "quote":
      return (
        <blockquote className="ez-md-quote" key={`block-${index}`}>
          {block.lines.map((line, lineIndex) => (
            <p key={`quote-${lineIndex}`}>{renderInline(line, highlight)}</p>
          ))}
        </blockquote>
      );
    default:
      return (
        <p className="ez-md-p" key={`block-${index}`}>
          {renderInline(block.text, highlight)}
        </p>
      );
  }
}

export function Markdown({ text, highlight }: MarkdownProps) {
  const blocks = parseBlocks(text);
  return (
    <div className="ez-md">
      {blocks.map((block, index) => renderBlock(block, index, highlight))}
    </div>
  );
}
