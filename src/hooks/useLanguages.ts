/**
 * The language table.
 *
 * `supportedLanguages()` never changes at runtime, so it is fetched once per
 * window and shared. The lookup map is what turns a raw `Language` value
 * (`"Simplified-Chinese"`) into something displayable.
 */

import { useEffect, useMemo, useState } from "react";

import * as api from "../api";
import type { Language, LanguageInfo } from "../types";

let cache: LanguageInfo[] | null = null;
let inflight: Promise<LanguageInfo[]> | null = null;

export async function loadLanguages(): Promise<LanguageInfo[]> {
  if (cache) {
    return cache;
  }
  if (!inflight) {
    inflight = api
      .supportedLanguages()
      .then((languages) => {
        cache = languages;
        return languages;
      })
      .finally(() => {
        inflight = null;
      });
  }
  return inflight;
}

export interface LanguageTable {
  languages: LanguageInfo[];
  /** Raw value -> descriptor. */
  get: (language: Language) => LanguageInfo | undefined;
  /** Display name honouring the app language, then the native name, then the raw value. */
  name: (language: Language) => string;
  /** BCP-47 code, for the flag emoji and language codes. */
  code: (language: Language) => string;
  loading: boolean;
}

export function useLanguages(): LanguageTable {
  const [languages, setLanguages] = useState<LanguageInfo[]>(cache ?? []);

  useEffect(() => {
    let active = true;
    void loadLanguages().then((loaded) => {
      if (active) {
        setLanguages(loaded);
      }
    });
    return () => {
      active = false;
    };
  }, []);

  const byValue = useMemo(() => {
    const map = new Map<Language, LanguageInfo>();
    for (const language of languages) {
      map.set(language.value, language);
    }
    return map;
  }, [languages]);

  const chineseUi = useMemo(() => {
    const first = byValue.get("Simplified-Chinese");
    // The app language follows the system locale, which the Rust side already
    // resolved when it built the descriptors: a Chinese UI is one whose first
    // entry has a Chinese name.
    return first?.chineseName.includes("简体") ?? false;
  }, [byValue]);

  return {
    languages,
    loading: languages.length === 0,
    get: (language) => byValue.get(language),
    code: (language) => byValue.get(language)?.code ?? language,
    name: (language) => {
      const info = byValue.get(language);
      if (!info) {
        return language;
      }
      return chineseUi ? info.chineseName : info.englishName;
    },
  };
}

/**
 * The languages a single service accepts. Falls back to the full table while
 * loading, so pickers never render empty.
 */
export function useServiceLanguages(identifier: string | null): LanguageTable {
  const all = useLanguages();
  const [forService, setForService] = useState<LanguageInfo[] | null>(null);

  useEffect(() => {
    if (!identifier) {
      setForService(null);
      return;
    }
    let active = true;
    void api
      .serviceLanguages(identifier)
      .then((languages) => {
        if (active) {
          setForService(languages);
        }
      })
      .catch(() => {
        if (active) {
          setForService(null);
        }
      });
    return () => {
      active = false;
    };
  }, [identifier]);

  const languages = forService ?? all.languages;
  const byValue = useMemo(() => {
    const map = new Map<Language, LanguageInfo>();
    for (const language of languages) {
      map.set(language.value, language);
    }
    return map;
  }, [languages]);

  return {
    languages,
    loading: languages.length === 0,
    get: (language) => byValue.get(language) ?? all.get(language),
    code: (language) => (byValue.get(language) ?? all.get(language))?.code ?? language,
    name: (language) => all.name(language),
  };
}
