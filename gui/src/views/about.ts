import { setState } from '../lib/state';
import { openUrl } from '@tauri-apps/plugin-opener';

const REPO_URL = 'https://github.com/realspqrk/autorename-pdf';
const LICENSE_URL = 'https://github.com/realspqrk/autorename-pdf/blob/main/LICENSE';
const PHRASEVAULT_URL = 'https://phrasevault.app';

const externalLinkIcon = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>`;

export function renderAboutView(root: HTMLElement): void {
  const version = typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.0.0';

  root.innerHTML = `
    <div class="about-scroll">
    <div class="about-view">

      <div class="about-header">
        <h1>AutoRename-PDF</h1>
        <p class="about-tagline">
          AI-powered PDF auto-renamer.<br>
          Extracts company name, date, and document type from PDFs.
        </p>
        <p class="about-version">v${version}</p>
      </div>

      <div class="about-section">
        <p class="about-section-title">Open Source</p>
        <p>
          Licensed under the
          <button class="about-link" data-href="${LICENSE_URL}">MIT License ${externalLinkIcon}</button>
        </p>
        <p style="margin-top: 0.5rem;">
          <button class="about-link" data-href="${REPO_URL}">View on GitHub ${externalLinkIcon}</button>
        </p>
      </div>

      <div class="about-section">
        <p class="about-section-title">Disclaimer</p>
        <p>
          This software is provided "as is", without warranty of any kind, express or implied.
          The authors are not liable for any claim, damages, or other liability arising from its use.
        </p>
      </div>

      <div class="about-support-card">
        <p>
          If you find this project useful, please consider supporting its development by checking out
        </p>
        <p>
          <button class="about-link" data-href="${PHRASEVAULT_URL}" style="font-size:var(--font-size-sm);">PhraseVault ${externalLinkIcon}</button>
        </p>
        <p>
          A text expander and snippet manager by the same developer. Your support helps keep this project free and maintained.
        </p>
      </div>

      <button class="btn btn-ghost btn-sm" id="btn-back-from-about">&larr; Back</button>
    </div>
    </div>
  `;

  // External link handler
  root.querySelectorAll<HTMLElement>('[data-href]').forEach((el) => {
    el.addEventListener('click', () => {
      const url = el.dataset.href;
      if (url) openUrl(url);
    });
  });

  document.getElementById('btn-back-from-about')?.addEventListener('click', () => {
    setState({ view: 'files' });
  });
}
