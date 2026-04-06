import { createContext, useContext, useState, useEffect, type ReactNode } from 'react';

type ColorMode = 'day' | 'night' | 'auto';

interface ThemeContextType {
  colorMode: ColorMode;
  setColorMode: (mode: ColorMode) => void;
  resolvedMode: 'day' | 'night';
}

const ThemeContext = createContext<ThemeContextType>(null!);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [colorMode, setColorMode] = useState<ColorMode>(() => {
    return (localStorage.getItem('forge-theme') as ColorMode) || 'auto';
  });

  const resolvedMode = colorMode === 'auto'
    ? (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'night' : 'day')
    : colorMode;

  useEffect(() => {
    localStorage.setItem('forge-theme', colorMode);
  }, [colorMode]);

  useEffect(() => {
    document.documentElement.setAttribute('data-color-mode', resolvedMode);
  }, [resolvedMode]);

  return (
    <ThemeContext.Provider value={{ colorMode, setColorMode, resolvedMode }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme() {
  return useContext(ThemeContext);
}
