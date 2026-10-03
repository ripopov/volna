import { defineCollection } from 'astro:content';
import { i18nLoader } from '@astrojs/starlight/loaders';
import { volnaDocsLoader } from '../scripts/docs-loader';
import { docsSchema, i18nSchema } from '@astrojs/starlight/schema';

export const collections = {
  docs: defineCollection({
    loader: volnaDocsLoader(),
    schema: docsSchema(),
  }),
  i18n: defineCollection({ loader: i18nLoader(), schema: i18nSchema() }),
};
