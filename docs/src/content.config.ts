import { defineCollection } from 'astro:content';
import { docsLoader, i18nLoader } from '@astrojs/starlight/loaders';
import { docsSchema, i18nSchema } from '@astrojs/starlight/schema';

export const collections = {
  docs: defineCollection({
    loader: {
      ...docsLoader(),
      async load(context) {
        await docsLoader().load(context);
        // Astro can log a render error and cache an entry without its HTML. Reject incomplete Markdown.
        for (const [id, entry] of context.store.entries()) {
          if (entry.filePath?.endsWith('.md') && !entry.rendered) {
            throw new Error(`Documentation failed to render: ${id}`);
          }
        }
      },
    },
    schema: docsSchema(),
  }),
  i18n: defineCollection({ loader: i18nLoader(), schema: i18nSchema() }),
};
