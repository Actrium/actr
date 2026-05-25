import { defineWorkload } from "@actrium/actr-workload";

const decoder = new TextDecoder();
const encoder = new TextEncoder();

export default defineWorkload({
  dispatch(envelope) {
    const input = decoder.decode(envelope.payload ?? new Uint8Array());
    return encoder.encode(`echo: ${input}`);
  },
});
