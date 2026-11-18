import { DISCLAIMER } from "../disclaimer";

/** Permanent disclaimer. No props, no dismiss, no condition. */
export function Disclaimer() {
  return (
    <footer role="contentinfo" className="disclaimer">
      {DISCLAIMER}
    </footer>
  );
}
