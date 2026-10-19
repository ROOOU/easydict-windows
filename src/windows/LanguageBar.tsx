/**
 * The language bar: `英语 ▾ ⇄ 简体中文 ▾`.
 *
 * Both pickers open a searchable list of the languages the backend reports.
 * "auto" is pinned to the top of the source list, the way the macOS app keeps
 * 自动检测 above the divider.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { useLanguages } from "../hooks/useLanguages";
import type { Language, LanguageInfo } from "../types";
import { ChevronDownIcon, SearchIcon, SwapIcon } from "./icons";

export interface LanguageBarProps {
  from: Language;
  to: Language;
  onChange: (next: { from?: Language; to?: Language }) => void;
  onSwap: () => void;
}

interface LanguagePickerProps {
  value: Language;
  label: string;
  languages: LanguageInfo[];
  displayName: (language: Language) => string;
  onSelect: (language: Language) => void;
}

function LanguagePicker({
  value,
  label,
  languages,
  displayName,
  onSelect,
}: LanguagePickerProps) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLSpanElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  const current = languages.find((language) => language.value === value);

  const filtered = useMemo(() => {
    const needle = search.trim().toLocaleLowerCase();
    if (!needle) {
      return languages;
    }
    return languages.filter((language) =>
      [language.chineseName, language.englishName, language.nativeName, language.code, language.value]
        .join(" ")
        .toLocaleLowerCase()
        .includes(needle),
    );
  }, [languages, search]);

  // Close on any click that lands outside the picker.
  useEffect(() => {
    if (!open) {
      return;
    }
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  useEffect(() => {
    if (open) {
      setSearch("");
      setActive(0);
      searchRef.current?.focus();
    }
  }, [open]);

  // Keep the highlighted row inside the visible part of the list.
  useEffect(() => {
    if (!open) {
      return;
    }
    const list = listRef.current;
    const option = list?.children[active] as HTMLElement | undefined;
    option?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  const choose = (language: Language) => {
    onSelect(language);
    setOpen(false);
  };

  return (
    <span className="ez-lang-picker" ref={rootRef}>
      <button
        type="button"
        className="ez-lang-button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={label}
        title={label}
        onClick={() => setOpen((previous) => !previous)}
      >
        {current?.flag ? <span className="ez-lang-flag">{current.flag}</span> : null}
        <span className="ez-lang-name">{displayName(value)}</span>
        <ChevronDownIcon size={12} />
      </button>

      {open ? (
        <div className="ez-lang-popover">
          <label className="ez-lang-search">
            <SearchIcon size={13} />
            <input
              ref={searchRef}
              value={search}
              placeholder="搜索语言"
              aria-label="搜索语言"
              onChange={(event) => {
                setSearch(event.target.value);
                setActive(0);
              }}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  setActive((index) => Math.min(index + 1, filtered.length - 1));
                } else if (event.key === "ArrowUp") {
                  event.preventDefault();
                  setActive((index) => Math.max(index - 1, 0));
                } else if (event.key === "Enter") {
                  event.preventDefault();
                  const chosen = filtered[active];
                  if (chosen) {
                    choose(chosen.value);
                  }
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  setOpen(false);
                }
              }}
            />
          </label>

          <div className="ez-lang-list ez-scroll" role="listbox" ref={listRef}>
            {filtered.length === 0 ? (
              <div className="ez-lang-empty">没有匹配的语言</div>
            ) : (
              filtered.map((language, index) => (
                <button
                  type="button"
                  key={language.value}
                  role="option"
                  aria-selected={language.value === value}
                  className={`ez-lang-option${index === active ? " ez-lang-option-active" : ""}${
                    language.value === value ? " ez-lang-option-selected" : ""
                  }`}
                  onMouseEnter={() => setActive(index)}
                  onClick={() => choose(language.value)}
                >
                  <span className="ez-lang-flag">{language.flag}</span>
                  <span className="ez-lang-option-name">{language.chineseName}</span>
                  <span className="ez-lang-code">{language.code}</span>
                </button>
              ))
            )}
          </div>
        </div>
      ) : null}
    </span>
  );
}

export function LanguageBar({ from, to, onChange, onSwap }: LanguageBarProps) {
  const languages = useLanguages();

  // "auto" first, then the backend's own order.
  const ordered = useMemo(() => {
    const copy = [...languages.languages];
    copy.sort((left, right) => {
      const leftAuto = left.value === "auto" ? 0 : 1;
      const rightAuto = right.value === "auto" ? 0 : 1;
      return leftAuto - rightAuto;
    });
    return copy;
  }, [languages.languages]);

  return (
    <div className="ez-language-bar">
      <LanguagePicker
        value={from}
        label="源语言"
        languages={ordered}
        displayName={languages.name}
        onSelect={(language) => onChange({ from: language })}
      />

      <button
        type="button"
        className="ez-icon-btn ez-swap-btn"
        title="交换语言"
        aria-label="交换语言"
        onClick={onSwap}
      >
        <SwapIcon size={15} />
      </button>

      <LanguagePicker
        value={to}
        label="目标语言"
        languages={ordered.filter((language) => language.value !== "auto")}
        displayName={languages.name}
        onSelect={(language) => onChange({ to: language })}
      />

      {languages.loading ? <span className="ez-spinner" aria-hidden="true" /> : null}
    </div>
  );
}
