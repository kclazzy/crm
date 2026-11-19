import { useState } from 'react';
import { useLocation } from 'react-router';
import { MessageSquareWarning } from 'lucide-react';
import { work } from '../work';
import { Button, ErrorText, Modal } from './ui';
import { useToast } from './Toast';

/** «Сообщить о проблеме»: пара слов — владельцу в колокольчик вместе со страницей и версией. */
export function ReportProblem({ className = '' }: { className?: string }) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const { pathname } = useLocation();
  const toast = useToast();
  return (
    <>
      <button onClick={() => setOpen(true)} className={className}>
        <MessageSquareWarning className="size-[18px]" /><span className="flex-1 text-left">Сообщить о проблеме</span>
      </button>
      <Modal open={open} onClose={() => setOpen(false)} title="Сообщить о проблеме">
        <p className="mb-3 text-sm text-slate-600 dark:text-slate-300">Опишите в паре слов, что не получается. Владелец увидит это в колокольчике вместе со страницей ({pathname}) и версией программы.</p>
        <textarea className="input min-h-28" value={text} onChange={(e) => setText(e.target.value)} maxLength={1000} autoFocus placeholder="Например: не открывается чат задачи «Договор поставки»" />
        <ErrorText>{error}</ErrorText>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="secondary" onClick={() => setOpen(false)}>Отмена</Button>
          <Button busy={busy} disabled={!text.trim()} onClick={async () => {
            setBusy(true);
            setError('');
            try { await work.reportProblem(text, pathname); setOpen(false); setText(''); toast('Отправлено владельцу'); } catch (x) { setError((x as Error).message); } finally { setBusy(false); }
          }}>Отправить</Button>
        </div>
      </Modal>
    </>
  );
}
