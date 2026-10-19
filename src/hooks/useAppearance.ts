/**
 * Applies `AppearanceType` to the document.
 *
 * The palette lives in CSS custom properties keyed off a `data-theme`
 * attribute, so switching the setting is a single attribute write and no
 * component re-renders.
 */

import { useEffect } from "react";

import { useSettings } from "./useSettings";
import type { AppearanceType } from "../types";

const ATTRIBUTE = "data-theme";

export function applyAppearance(appearance: AppearanceType) {
  document.documentElement.setAttribute(ATTRIBUTE, appearance);
}

export function useAppearance() {
  const { settings } = useSettings();

  useEffect(() => {
    if (settings) {
      applyAppearance(settings.appearanceType);
    }
  }, [settings?.appearanceType]);

  return settings?.appearanceType ?? "followSystem";
}

/** The clamped font ratio the result text is scaled by. */
export function useFontRatio(): number {
  const { settings } = useSettings();
  const index = settings?.fontSizeOptionIndex ?? 0;
  const ratios = [1.0, 1.1, 1.2, 1.3, 1.4];
  return ratios[Math.min(Math.max(index, 0), ratios.length - 1)];
}
