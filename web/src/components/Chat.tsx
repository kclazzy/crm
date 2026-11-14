import { type FormEvent, useCallback, useEffect, useRef, useState } from 'react';
import { CornerUpLeft, FileText, Loader2, Paperclip, Pencil, Send, X } from 'lucide-react';
import { useAuth } from '../auth';
import { useLive, useLiveReload } from '../live';
import { type Attachment, type Message, work } from '../work';
import { Avatar } from './ui';
import { Md } from './work';
import { bytes } from '../api';

function time(ms: number) {
  const d = new Date(ms);
  const today = new Date().toDateString() === d.toDateString();
  return d.toLocaleString('ru-RU', today ? { hour: '2-digit', minute: '2-digit' } : { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

function Files({ files }: { files: Attachment[] }) {
  if (!files.length) return null;
  return (
    <div className="mt-1.5 flex flex-wrap gap-2">
      {files.map((f) =>
        f.mime.startsWith('image/') ? (
          <a key={f.sha256} href={`/api/attachments/${f.sha256}`} target="_blank" rel="noreferrer">
            <img src={`/api/attachments/${f.sha256}`} alt={f.file_name} className="max-h-48 max-w-64 rounded-lg border border-slate-200 object-cover dark:border-slate-700" loading="lazy" />
          </a>
        ) : (
          <a key={f.sha256} href={`/api/attachments/${f.sha256}?download=${f.mime === 'application/pdf' ? 'false' : 'true'}`} target="_blank" rel="noreferrer"
            className="inline-flex items-center gap-2 rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm hover:border-brand-500 dark:border-slate-700 dark:bg-slate-900">
            <FileText className="size-4 text-brand-500" /><span className="max-w-48 truncate">{f.file_name}</span><span className="text-xs text-slate-500">{bytes(f.size_bytes)}</span>
          </a>
        ),
      )}
    </div>
  );
}

/** Чат задачи: системные записи, ответы, правка своих 15 минут, файлы, «Новые сообщения». */
export function Chat({ taskId, canWrite }: { taskId: string; canWrite: boolean }) {
  const { me } = useAuth();
  const { refreshUnread } = useLive();
  const [messages, setMessages] = useState<Message[] | null>(null);
  const [firstUnread, setFirstUnread] = useState<string | null>(null);
  const [text, setText] = useState('');
  const [reply, setReply] = useState<Message | null>(null);
  const [editing, setEditing] = useState<Message | null>(null);
  const [files, setFiles] = useState<Attachment[]>([]);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState('');
  const list = useRef<HTMLDivElement>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const lastMarked = useRef('');

  const load = useCallback(async () => {
    const r = await work.messages(taskId);
    setMessages(r.messages);
    // Разделитель — над первым чужим сообщением после прочитанного; запоминаем при открытии.
    setFirstUnread((prev) => prev ?? r.messages.find((m) => m.sender_id !== me?.id && (!r.last_read || m.id > r.last_read))?.id ?? '');
  }, [taskId, me?.id]);

  useEffect(() => {
    setMessages(null);
    setFirstUnread(null);
    lastMarked.current = '';
    stick.current = true;
    load().catch((e) => setError(e.message));
  }, [load]);
  useLiveReload(() => void load().catch(() => {}), taskId);

  // Прокрутка вниз, если пользователь и так был внизу.
  useEffect(() => {
    if (stick.current) bottom.current?.scrollIntoView({ block: 'end' });
  }, [messages]);

  // Прочитано — когда виден низ чата.
  useEffect(() => {
    const el = bottom.current;
    if (!el || !messages?.length) return;
    const obs = new IntersectionObserver(([e]) => {
      stick.current = e.isIntersecting;
      const last = messages[messages.length - 1]?.id;
      if (e.isIntersecting && last && last !== lastMarked.current && document.visibilityState === 'visible') {
        lastMarked.current = last;
        work.read(taskId, last).then(refreshUnread).catch(() => {});
      }
    });
    obs.observe(el);
    return () => obs.disconnect();
  }, [messages, taskId, refreshUnread]);

  async function attach(list: FileList | File[]) {
    setUploading(true);
    setError('');
    try {
      for (const f of Array.from(list)) {
        const a = await work.upload(f);
        setFiles((x) => [...x, a]);
      }
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setUploading(false);
    }
  }

  async function send(e?: FormEvent) {
    e?.preventDefault();
    if (!text.trim() && !files.length) return;
    setError('');
    try {
      if (editing) {
        await work.edit(editing.id, text);
        setEditing(null);
      } else {
        await work.post(taskId, text, reply?.id ?? null, files.map((f) => ({ sha256: f.sha256, file_name: f.file_name })));
        setReply(null);
        setFiles([]);
      }
      setText('');
      stick.current = true;
      await load();
    } catch (x) {
      setError((x as Error).message);
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col"
      onDragOver={(e) => { if (canWrite) e.preventDefault(); }}
      onDrop={(e) => { if (canWrite && e.dataTransfer.files.length) { e.preventDefault(); void attach(e.dataTransfer.files); } }}>
      <div ref={list} className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 py-4">
        {messages === null && <Loader2 className="mx-auto size-5 animate-spin text-slate-400" />}
        {messages?.length === 0 && <p className="m-auto text-sm text-slate-500">Сообщений пока нет.</p>}
        {messages?.map((m) => (
          <div key={m.id}>
            {m.id === firstUnread && (
              <div className="my-2 flex items-center gap-3 text-xs font-semibold text-brand-600"><span className="h-px flex-1 bg-brand-200 dark:bg-brand-500/30" />Новые сообщения<span className="h-px flex-1 bg-brand-200 dark:bg-brand-500/30" /></div>
            )}
            {m.kind === 'system' ? (
              <p className="text-center text-xs text-slate-500">{m.content} · {time(m.created_at)}</p>
            ) : (
              <div className={`group flex gap-2.5 ${m.sender_id === me?.id ? 'flex-row-reverse' : ''}`}>
                <Avatar name={m.sender_name} size="sm" />
                <div className={`max-w-[80%] rounded-2xl px-3.5 py-2 ${m.sender_id === me?.id ? 'bg-brand-50 dark:bg-brand-500/15' : 'bg-slate-100 dark:bg-slate-800'}`}>
                  <p className="text-xs font-semibold text-slate-500">{m.sender_name} · {time(m.created_at)}{m.edited_at ? ' · изменено' : ''}</p>
                  {m.reply_to && (
                    <p className="mt-1 border-l-2 border-brand-400 pl-2 text-xs text-slate-500"><b>{m.reply_to.sender_name}:</b> {m.reply_to.excerpt}</p>
                  )}
                  <Md text={m.content} className="mt-0.5 select-text" />
                  <Files files={m.attachments} />
                  {canWrite && (
                    <div className="mt-1 hidden gap-3 text-xs text-slate-500 group-hover:flex">
                      <button onClick={() => { setReply(m); setEditing(null); }} className="inline-flex items-center gap-1 hover:text-brand-600"><CornerUpLeft className="size-3" />Ответить</button>
                      {m.can_edit && <button onClick={() => { setEditing(m); setReply(null); setText(m.content); }} className="inline-flex items-center gap-1 hover:text-brand-600"><Pencil className="size-3" />Исправить</button>}
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
        ))}
        <div ref={bottom} className="h-px" />
      </div>
      {canWrite ? (
        <form onSubmit={send} className="border-t border-slate-200 p-3 dark:border-slate-800">
          {(reply || editing) && (
            <div className="mb-2 flex items-center gap-2 rounded-lg bg-slate-100 px-3 py-1.5 text-xs dark:bg-slate-800">
              <span className="flex-1 truncate">{editing ? 'Исправление сообщения' : <>Ответ <b>{reply?.sender_name}</b>: {reply?.content}</>}</span>
              <button type="button" onClick={() => { setReply(null); setEditing(null); setText(''); }}><X className="size-3.5" /></button>
            </div>
          )}
          {files.length > 0 && (
            <div className="mb-2 flex flex-wrap gap-1.5">
              {files.map((f) => (
                <span key={f.sha256} className="inline-flex items-center gap-1 rounded-lg bg-slate-100 px-2 py-1 text-xs dark:bg-slate-800">
                  {f.file_name}<button type="button" onClick={() => setFiles((x) => x.filter((y) => y !== f))}><X className="size-3" /></button>
                </span>
              ))}
            </div>
          )}
          {error && <p className="mb-2 text-xs text-red-600">{error}</p>}
          <div className="flex items-end gap-2">
            <label className="cursor-pointer rounded-xl p-2.5 text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-800" title="Прикрепить файл (до 100 МБ)">
              {uploading ? <Loader2 className="size-5 animate-spin" /> : <Paperclip className="size-5" />}
              <input type="file" multiple hidden onChange={(e) => e.target.files && void attach(e.target.files)} />
            </label>
            <textarea
              className="input max-h-40 min-h-11 flex-1 resize-none py-2.5"
              rows={1}
              placeholder="Сообщение… (Enter — отправить, Shift+Enter — новая строка)"
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  void send();
                }
              }}
            />
            <button type="submit" className="rounded-xl bg-brand-600 p-2.5 text-white hover:bg-brand-700 disabled:opacity-50" disabled={!text.trim() && !files.length} title="Отправить">
              <Send className="size-5" />
            </button>
          </div>
        </form>
      ) : (
        <p className="border-t border-slate-200 p-3 text-center text-xs text-slate-500 dark:border-slate-800">Писать в чат могут участники задачи.</p>
      )}
    </div>
  );
}
