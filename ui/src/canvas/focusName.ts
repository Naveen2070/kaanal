// A node just added from the palette: its name field in the details panel takes focus once, so the
// person can type a real name at once (review C3). Set by `useAddNode`, read and cleared by Controls.

export const focusName: { pending: string | null } = { pending: null };
