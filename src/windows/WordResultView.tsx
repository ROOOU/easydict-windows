/**
 * The dictionary body of a card.
 *
 * Mirrors Easydict's `EZWordResultView`: phonetics with a speaker per accent,
 * then one block per part of speech, then the word's inflections, the "simple
 * words" the dictionary cross-references, tags, etymology and the
 * synonym/antonym/collocation tables.
 */

import type { Language, TranslatePart, TranslateWordResult } from "../types";
import { SpeakerIcon } from "./icons";

export interface SpeakOptions {
  language?: Language;
  url?: string | null;
}

export interface WordResultViewProps {
  word: TranslateWordResult;
  /** The word that was queried, used for the TTS fallback. */
  queryText: string;
  onSpeak: (text: string, options?: SpeakOptions) => void;
  /** Re-queries a word from `simpleWords`; omitted when the list is read-only. */
  onQueryWord?: (text: string) => void;
}

function PartRows({ parts }: { parts: TranslatePart[] }) {
  return (
    <div className="ez-parts">
      {parts.map((part, index) => (
        <div className="ez-part" key={`part-${index}`}>
          {part.part ? <span className="ez-part-label">{part.part}</span> : null}
          <span className="ez-part-means">
            {part.means.map((mean, meanIndex) => (
              <span className="ez-mean" key={`mean-${meanIndex}`}>
                {mean}
              </span>
            ))}
          </span>
        </div>
      ))}
    </div>
  );
}

export function WordResultView({
  word,
  queryText,
  onSpeak,
  onQueryWord,
}: WordResultViewProps) {
  const phonetics = word.phonetics ?? [];
  const parts = word.parts ?? [];
  const exchanges = word.exchanges ?? [];
  const simpleWords = word.simpleWords ?? [];
  const tags = word.tags ?? [];
  const related: Array<{ label: string; parts: TranslatePart[] }> = [
    { label: "同义词", parts: word.synonyms ?? [] },
    { label: "反义词", parts: word.antonyms ?? [] },
    { label: "搭配", parts: word.collocation ?? [] },
  ].filter((section) => section.parts.length > 0);

  const hasBody =
    phonetics.length > 0 ||
    parts.length > 0 ||
    exchanges.length > 0 ||
    simpleWords.length > 0 ||
    tags.length > 0 ||
    Boolean(word.etymology) ||
    related.length > 0;

  if (!hasBody) {
    return null;
  }

  return (
    <div className="ez-word">
      {phonetics.length > 0 ? (
        <div className="ez-phonetics">
          {phonetics.map((phonetic, index) => (
            <span className="ez-phonetic" key={`phonetic-${index}`}>
              <span className="ez-phonetic-name">
                {phonetic.name ?? phonetic.accent ?? ""}
              </span>
              {phonetic.value ? (
                <span className="ez-phonetic-value">{phonetic.value}</span>
              ) : null}
              <button
                type="button"
                className="ez-icon-btn ez-phonetic-speak"
                title="发音"
                aria-label="发音"
                onClick={() =>
                  onSpeak(phonetic.word || queryText, {
                    language: phonetic.language,
                    url: phonetic.speakUrl,
                  })
                }
              >
                <SpeakerIcon size={13} />
              </button>
            </span>
          ))}
        </div>
      ) : null}

      {parts.length > 0 ? <PartRows parts={parts} /> : null}

      {exchanges.length > 0 ? (
        <div className="ez-exchanges">
          {exchanges.map((exchange, index) => (
            <div className="ez-exchange" key={`exchange-${index}`}>
              <span className="ez-exchange-name">{exchange.name}</span>
              <span className="ez-exchange-words">{exchange.words.join(" / ")}</span>
            </div>
          ))}
        </div>
      ) : null}

      {simpleWords.length > 0 ? (
        <div className="ez-simple-words">
          {simpleWords.map((simple, index) => {
            const means = simple.meansText;
            const text =
              simple.showPartMeans && simple.part ? `${simple.part} ${means}` : means;
            return (
              <div className="ez-simple-word" key={`simple-${index}`}>
                <button
                  type="button"
                  className="ez-simple-word-word ez-link"
                  title="查询该词"
                  onClick={() => onQueryWord?.(simple.word)}
                >
                  {simple.word}
                </button>
                <span className="ez-simple-word-means">{text}</span>
              </div>
            );
          })}
        </div>
      ) : null}

      {tags.length > 0 ? (
        <div className="ez-tags">
          {tags.map((tag, index) => (
            <span className="ez-badge" key={`tag-${index}`}>
              {tag}
            </span>
          ))}
        </div>
      ) : null}

      {related.map((section) => (
        <div className="ez-related" key={section.label}>
          <span className="ez-related-label">{section.label}</span>
          <div className="ez-related-body">
            <PartRows parts={section.parts} />
          </div>
        </div>
      ))}

      {word.etymology ? (
        <div className="ez-etymology">
          <span className="ez-related-label">词源</span>
          <p className="ez-etymology-text">{word.etymology}</p>
        </div>
      ) : null}
    </div>
  );
}
