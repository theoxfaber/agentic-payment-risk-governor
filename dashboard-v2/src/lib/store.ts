import { create } from 'zustand'

type State = {
  filter: 'all' | 'review' | 'blocked' | 'allow'
  selectedId: string | null
  setFilter: (f: State['filter']) => void
  setSelected: (id: string | null) => void
}

export const useStore = create<State>((set) => ({
  filter: 'all',
  selectedId: null,
  setFilter: (filter) => set({ filter }),
  setSelected: (selectedId) => set({ selectedId }),
}))
