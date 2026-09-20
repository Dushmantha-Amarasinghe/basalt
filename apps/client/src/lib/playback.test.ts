import { describe, expect, it } from 'vitest'
import { isMediaFile } from './playback'

describe('isMediaFile', () => {
  /// The report: a PDF and a zip both offered "Play in your player".
  it('does not offer to play a document or an archive', () => {
    for (const name of [
      'මූලික_විමසීම්_වගු_මාර්ගෝපදේශය.pdf',
      'Wireframe_Diagrams.zip',
      'notes.txt',
      'photo.jpg',
      'setup.exe',
      'no-extension',
    ]) {
      expect(isMediaFile(name)).toBe(false)
    }
  })

  /// These used to need an external player, because Chromium would not touch
  /// them. The built-in player is mpv now and opens them itself, so there is
  /// one answer here rather than two.
  it('includes the containers Chromium would never open', () => {
    for (const name of ['film.avi', 'clip.wmv', 'old.flv', 'rip.vob']) {
      expect(isMediaFile(name)).toBe(true)
    }
  })

  it('offers video and audio the obvious way', () => {
    for (const name of ['a.mkv', 'b.mp4', 'c.mp3', 'd.flac', 'E.MKV']) {
      expect(isMediaFile(name)).toBe(true)
    }
  })
})
