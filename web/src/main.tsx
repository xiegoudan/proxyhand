import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App.tsx';
import { I18nProvider } from './i18n.tsx';
import { TooltipProvider } from './components/ui/tooltip.tsx';
import './index.css';

const container = document.getElementById('root');
if (!container) throw new Error('#root missing');
createRoot(container).render(
  <StrictMode>
    <I18nProvider>
      <TooltipProvider>
        <App />
      </TooltipProvider>
    </I18nProvider>
  </StrictMode>,
);
