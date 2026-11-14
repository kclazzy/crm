import { createContext, type ReactNode, useContext, useEffect, useState } from 'react';
import { api, type Me } from './api';

interface AuthState {
  me: Me | null;
  loading: boolean;
  setMe: (me: Me | null) => void;
}

const Ctx = createContext<AuthState>({ me: null, loading: true, setMe: () => {} });

export function AuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null>(null);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    api.me().then(setMe).catch(() => setMe(null)).finally(() => setLoading(false));
  }, []);
  return <Ctx.Provider value={{ me, loading, setMe }}>{children}</Ctx.Provider>;
}

export const useAuth = () => useContext(Ctx);
