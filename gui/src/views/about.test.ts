import { beforeEach, describe, expect, it, vi } from 'vitest';

const { openUrl } = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }));

import { renderAboutView } from './about';
import { getState, setState } from '../lib/state';

const URLS = [
  'https://github.com/realspqrk/autorename-pdf',
  'https://github.com/realspqrk/autorename-pdf/blob/main/LICENSE',
  'https://phrasevault.app',
];

describe('about view', () => {
  beforeEach(() => {
    openUrl.mockReset();
    document.body.innerHTML = '<div id="root"></div>';
    setState({ view: 'about' });
  });

  it('opens GitHub, license, and PhraseVault via data-href', () => {
    renderAboutView(document.getElementById('root')!);
    const buttons = [...document.querySelectorAll<HTMLElement>('[data-href]')];
    const hrefs = buttons.map((el) => el.dataset.href);
    expect(hrefs).toEqual(expect.arrayContaining(URLS));
    for (const url of URLS) {
      buttons.find((el) => el.dataset.href === url)!.click();
      expect(openUrl).toHaveBeenCalledWith(url);
    }
    expect(openUrl).toHaveBeenCalledTimes(3);
  });

  it('Back returns to files', () => {
    renderAboutView(document.getElementById('root')!);
    document.getElementById('btn-back-from-about')!.click();
    expect(getState().view).toBe('files');
  });
});
