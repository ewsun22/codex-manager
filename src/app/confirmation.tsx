import { useCallback, useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react";
import { ConfirmationAction, ConfirmationState, type ConfirmationRequest } from "./confirmation-state.ts";
import { t } from "./i18n-core.ts";

export function useConfirmation() {
  const state = useRef(new ConfirmationState()).current;
  const actions = useRef(new ConfirmationAction()).current;
  const current = useSyncExternalStore(state.subscribe, state.getSnapshot, state.getSnapshot);
  const element = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    actions.activate();
    return () => { actions.deactivate(); state.cancel(); };
  }, [actions, state]);
  const runAction = useCallback((action: () => Promise<void>, request?: ConfirmationRequest) =>
    actions.run(async () => { await action(); return true; }, request ? () => state.request(request) : undefined), [actions, state]);
  useEffect(() => {
    const dialog = element.current;
    if (!dialog || !current) return;
    try { dialog.showModal(); }
    catch { state.answer(current.id, false); }
    return () => { if (dialog.open) dialog.close(); };
  }, [current, state]);

  const dialog = current ? (
    <dialog key={current.id} ref={element} className="confirmation-dialog" aria-labelledby="confirmation-title" aria-describedby="confirmation-message"
      onCancel={(event) => { event.preventDefault(); state.answer(current.id, false); }}
      onClose={() => state.answer(current.id, false)}>
      <form onSubmit={(event) => { event.preventDefault(); state.answer(current.id, true); }}>
        <h2 id="confirmation-title">{current.title}</h2>
        <p id="confirmation-message">{current.message}</p>
        <div className="confirmation-actions">
          <button type="button" className="button button-secondary" autoFocus onClick={() => state.answer(current.id, false)}>{t("取消")}</button>
          <button type="submit" className={`button ${current.destructive ? "button-danger" : "button-primary"}`} data-confirmation-action="accept">{current.confirmLabel}</button>
        </div>
      </form>
    </dialog>
  ) : null;
  return { confirm: state.request, runAction, dialog, confirming: current !== null };
}
