import * as React from "react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  readLocalePreference,
  resolveLocale,
  writeLocalePreference,
  setCurrentLocale,
  t,
  type Locale,
  type LocalePreference,
} from "./i18n-core.ts";

export type { Locale, LocalePreference } from "./i18n-core.ts";

export function LanguageSwitcher(): React.ReactElement {
  const { locale, preference, setPreference } = useI18n();
  return (
    <div className="language-switcher" role="group" aria-label={t("界面语言")}>
      <button type="button" className={locale === "zh-CN" ? "is-active" : ""} aria-pressed={locale === "zh-CN"} onClick={() => setPreference("zh-CN")}>中文</button>
      <button type="button" className={locale === "en-US" ? "is-active" : ""} aria-pressed={locale === "en-US"} onClick={() => setPreference("en-US")}>English</button>
      {preference !== "auto" ? <button type="button" className="language-auto" onClick={() => setPreference("auto")} title={t("跟随设备语言")} aria-label={t("跟随设备语言")}>↺</button> : null}
    </div>
  );
}

interface I18nContextValue {
  locale: Locale;
  preference: LocalePreference;
  setPreference: (preference: LocalePreference) => void;
}

const I18nContext = React.createContext<I18nContextValue | null>(null);

export function I18nProvider({ children }: { children: ReactNode }): React.ReactElement {
  const [preference, setPreferenceState] = useState<LocalePreference>(() => readLocalePreference());
  const [locale, setLocale] = useState<Locale>(() => resolveLocale(preference));
  const setPreference = useCallback((next: LocalePreference) => {
    writeLocalePreference(next);
    setPreferenceState(next);
    const resolved = resolveLocale(next);
    setLocale(resolved);
    setCurrentLocale(resolved);
  }, []);
  useEffect(() => { setCurrentLocale(locale); }, [locale]);
  const value = useMemo(() => ({ locale, preference, setPreference }), [locale, preference, setPreference]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const value = React.useContext(I18nContext);
  if (!value) throw new Error("useI18n must be called inside I18nProvider.");
  return value;
}
