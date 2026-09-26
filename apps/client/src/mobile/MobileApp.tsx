import type { AppModel } from '@/App'

/** The phone and tablet app. */
export function MobileApp({ model }: { model: AppModel }): React.JSX.Element {
  return <div className="p-6 text-text">{model.vault.status?.vault ?? 'Basalt'}</div>
}
