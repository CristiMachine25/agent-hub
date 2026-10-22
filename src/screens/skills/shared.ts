import type { DirectorySkill } from '@shared/types'
import { api, errorText } from '@/api'
import { useStore } from '@/store'
import { featuredFor, folderOf } from '@/lib/skills'

/** What a row needs: a directory result, or a featured entry before its counts are known. */
export type Listing = { id: string; name: string; source: string; installs?: number; blurb?: string }

export const toolList = new Intl.ListFormat('en', { type: 'conjunction' })

export const fromId = (id: string): Listing => ({ id, name: folderOf(id), source: id.slice(0, id.lastIndexOf('/')), blurb: featuredFor(id)?.blurb })
export const fromResult = (s: DirectorySkill): Listing => ({ ...s, blurb: featuredFor(s.id)?.blurb })

export function openUrl(url: string) {
  api.system.openUrl(url).catch((e) => useStore.getState().toast({ kind: 'error', title: "Couldn't open the link", body: errorText(e) }))
}
