import { createContext, type ReactNode, useCallback, useContext, useRef, useState } from 'react';
import { Button, Modal } from './ui';

export interface ConfirmOptions {
  title: string;
  text: ReactNode;
  ok: string;
  danger?: boolean;
}

const Ctx = createContext<(o: ConfirmOptions) => Promise<boolean>>(async () => false);

/** Окно подтверждения в стиле CRM вместо системного confirm(). */
export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [opts, setOpts] = useState<ConfirmOptions | null>(null);
  const resolve = useRef<(v: boolean) => void>(() => {});
  const ask = useCallback((o: ConfirmOptions) => new Promise<boolean>((r) => { resolve.current = r; setOpts(o); }), []);
  const done = (v: boolean) => { setOpts(null); resolve.current(v); };
  return (
    <Ctx.Provider value={ask}>
      {children}
      <Modal open={!!opts} onClose={() => done(false)} title={opts?.title ?? ''}>
        <div className="text-sm text-slate-600 dark:text-slate-300">{opts?.text}</div>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => done(false)}>Отмена</Button>
          <Button variant={opts?.danger ? 'danger' : 'primary'} onClick={() => done(true)} autoFocus>{opts?.ok}</Button>
        </div>
      </Modal>
    </Ctx.Provider>
  );
}

export const useConfirm = () => useContext(Ctx);
