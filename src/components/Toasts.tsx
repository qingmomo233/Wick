export type ToastItem = { id: string; text: string };

export default function Toasts({ items }: { items: ToastItem[] }) {
  if (items.length === 0) return null;
  return (
    <div className="toasts">
      {items.map((t) => (
        <div key={t.id} className="toast">{t.text}</div>
      ))}
    </div>
  );
}
