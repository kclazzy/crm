import type { ReactNode } from 'react';
import { CheckCircle2, MessagesSquare, ShieldCheck } from 'lucide-react';

/** Экран входа: слева — бренд и преимущества, справа — форма. */
export function AuthLayout({ title, subtitle, children }: { title: string; subtitle: string; children: ReactNode }) {
  return (
    <div className="grid min-h-dvh lg:grid-cols-[1.05fr_1fr]">
      <aside className="relative hidden overflow-hidden bg-gradient-to-br from-brand-600 via-indigo-600 to-violet-700 p-12 text-white lg:flex lg:flex-col">
        <div className="absolute -right-24 -top-24 size-96 rounded-full bg-white/10 blur-3xl" />
        <div className="absolute -bottom-32 -left-16 size-[28rem] rounded-full bg-violet-400/20 blur-3xl" />
        <div className="relative flex items-center gap-3 text-lg font-semibold">
          <img src="/logo.png" alt="" className="size-9 rounded-xl ring-2 ring-white/30" /> CRM
        </div>
        <div className="relative mt-auto max-w-md">
          <h2 className="text-4xl font-bold leading-tight tracking-tight">Клиенты, задачи и общение команды — в одном месте</h2>
          <ul className="mt-10 space-y-5 text-white/90">
            <li className="flex gap-3"><CheckCircle2 className="size-6 shrink-0 text-white" />Задачи по каждому клиенту с этапами и сроками</li>
            <li className="flex gap-3"><MessagesSquare className="size-6 shrink-0 text-white" />Чат внутри каждой задачи — история не теряется</li>
            <li className="flex gap-3"><ShieldCheck className="size-6 shrink-0 text-white" />Данные хранятся на вашем сервере, соединение зашифровано</li>
          </ul>
        </div>
      </aside>
      <main className="flex items-center justify-center p-6 sm:p-12">
        <div className="animate-rise w-full max-w-sm">
          <img src="/logo.png" alt="" className="mb-8 size-12 rounded-2xl shadow-lg shadow-brand-600/30 lg:hidden" />
          <h1 className="text-3xl font-bold tracking-tight">{title}</h1>
          <p className="mt-2 text-slate-500 dark:text-slate-400">{subtitle}</p>
          <div className="mt-8">{children}</div>
        </div>
      </main>
    </div>
  );
}
