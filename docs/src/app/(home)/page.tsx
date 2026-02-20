import Link from "next/link";
import Image from "next/image";

export default function HomePage() {
  return (
    <div className="flex flex-col items-center justify-center text-center flex-1 px-4 py-16 gap-8">
      <Image
        src="/native-window.webp"
        alt="native-window"
        width={280}
        height={280}
        priority
        className="rounded-2xl"
      />
      <div className="flex flex-col items-center gap-4 max-w-xl">
        <h1 className="text-4xl font-extrabold tracking-tight sm:text-5xl">
          native-window
        </h1>
        <p className="text-lg text-fd-muted-foreground">
          Native OS webview windows for Bun and Node.js — no Electron, no
          bundled Chromium.
        </p>
      </div>
      <div className="flex flex-row gap-3">
        <Link
          href="/docs/getting-started"
          className="inline-flex items-center justify-center rounded-full bg-fd-primary px-6 py-2.5 text-sm font-medium text-fd-primary-foreground shadow-sm transition-colors hover:bg-fd-primary/90"
        >
          Getting Started
        </Link>
        <a
          href="https://github.com/fcannizzaro/native-window"
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center justify-center rounded-full border border-fd-border px-6 py-2.5 text-sm font-medium text-fd-foreground transition-colors hover:bg-fd-accent hover:text-fd-accent-foreground"
        >
          GitHub
        </a>
      </div>
    </div>
  );
}
