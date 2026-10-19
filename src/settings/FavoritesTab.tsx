/**
 * 收藏 — the saved query records.
 *
 * The store is the source of truth, so every mutation re-queries it rather than
 * patching a local copy: a delete and a re-query is the only way to be sure the
 * list matches what will be there on the next launch.
 */

import { useCallback, useEffect, useState } from "react";

import * as api from "../api";
import { useLanguages } from "../hooks/useLanguages";
import type { QueryRecord } from "../types";
import { Banner, Icon, Section, errorMessage, formatTimestamp } from "./components";

export function FavoritesTab() {
  const languages = useLanguages();
  const [records, setRecords] = useState<QueryRecord[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setRecords(await api.favorites());
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const remove = async (record: QueryRecord) => {
    setBusy(record.queryText);
    try {
      await api.removeFavorite(record);
      await load();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const requery = async (record: QueryRecord) => {
    try {
      await api.query({
        text: record.queryText,
        from: record.fromLanguage,
        to: record.toLanguage,
        windowType: "main",
      });
      await api.showWindow("main");
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  return (
    <>
      {error !== null && (
        <Banner kind="error" onDismiss={() => setError(null)}>
          {error}
        </Banner>
      )}

      <Section
        title="收藏"
        description="收藏的查询记录，可重新查询或删除。"
        actions={
          <button type="button" className="ez-btn" onClick={() => void load()}>
            <Icon name="refresh" size={12} />
            刷新
          </button>
        }
      >
        {records === null ? (
          <div className="ezy-empty-block">
            <span className="ez-spinner" />
            正在读取收藏…
          </div>
        ) : records.length === 0 ? (
          <div className="ezy-empty-block">还没有收藏的查询。</div>
        ) : (
          records.map((record) => {
            const key = `${record.queryText}|${record.timestamp}`;
            return (
              <div key={key} className="ezy-record">
                <div className="ezy-record-main">
                  <span className="ezy-record-text ez-selectable">{record.queryText}</span>
                  {(record.translatedText ?? "") !== "" && (
                    <span className="ezy-record-translation ez-selectable">
                      {record.translatedText}
                    </span>
                  )}
                  <span className="ezy-record-meta">
                    {languages.name(record.fromLanguage)} → {languages.name(record.toLanguage)}
                    {" · "}
                    {formatTimestamp(record.timestamp)}
                  </span>
                </div>
                <div className="ezy-record-actions">
                  <button
                    type="button"
                    className="ez-btn"
                    onClick={() => void requery(record)}
                  >
                    <Icon name="search" size={12} />
                    重新查询
                  </button>
                  <button
                    type="button"
                    className="ez-icon-btn"
                    aria-label="删除收藏"
                    title="删除收藏"
                    disabled={busy === record.queryText}
                    onClick={() => void remove(record)}
                  >
                    <Icon name="trash" size={13} />
                  </button>
                </div>
              </div>
            );
          })
        )}
      </Section>
    </>
  );
}
