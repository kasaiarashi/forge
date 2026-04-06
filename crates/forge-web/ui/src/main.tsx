import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import { ThemeProvider as PrimerTheme, BaseStyles } from '@primer/react';
import { AuthProvider } from './context/AuthContext';
import { ThemeProvider, useTheme } from './context/ThemeContext';
import './index.css';
import App from './App';

function ThemedApp() {
  const { resolvedMode } = useTheme();
  return (
    <PrimerTheme colorMode={resolvedMode}>
      <BaseStyles>
        <AuthProvider>
          <App />
        </AuthProvider>
      </BaseStyles>
    </PrimerTheme>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <BrowserRouter>
      <ThemeProvider>
        <ThemedApp />
      </ThemeProvider>
    </BrowserRouter>
  </StrictMode>,
);
