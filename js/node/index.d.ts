/** A custom filter: called synchronously as `{{ value | name: arg1, arg2 }}`. */
export type Filter = (value: any, ...args: any[]) => any;

/**
 * Renders `template`.
 *
 * @param context  Variables, as an object or as a JSON string.
 * @param partials Named templates for `{% include %}` and `{% extends %}`.
 * @param filters  Custom filters; only the object's own properties are visible to templates.
 * @throws Error with a message that includes the template line.
 */
export function render(
  template: string,
  context?: Record<string, unknown> | string | null,
  partials?: Record<string, string> | null,
  filters?: Record<string, Filter> | null,
): string;
