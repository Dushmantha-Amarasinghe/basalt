import { describe, expect, it } from 'vitest'
import { keepsNativeMenu } from './nativeMenu'

describe('keepsNativeMenu', () => {
  it('leaves text fields their cut, copy and paste', () => {
    // Removing these would cost a real affordance to fix a cosmetic one: the
    // menu in a text field has no browser commands in it, only editing ones.
    expect(keepsNativeMenu('input')).toBe(true)
    expect(keepsNativeMenu('TEXTAREA')).toBe(true)
    expect(keepsNativeMenu('div', true)).toBe(true)
  })

  it('takes it away everywhere else', () => {
    // These are the places WebView2 was offering Back, Refresh, Save as and
    // Print — over the top of the app's own menu.
    for (const tag of ['div', 'button', 'span', 'body', 'img', 'video']) {
      expect(keepsNativeMenu(tag)).toBe(false)
    }
  })
})
