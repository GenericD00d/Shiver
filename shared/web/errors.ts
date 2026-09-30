/** Tauri rejects with the rust error's user-facing message, which is already worth showing. */
export const errorMessage = (error: unknown) =>
  typeof error === 'string' ? error : error instanceof Error ? error.message : 'Something went wrong';
