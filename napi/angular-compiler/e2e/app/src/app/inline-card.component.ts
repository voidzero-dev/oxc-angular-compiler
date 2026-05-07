import { Component } from '@angular/core'

@Component({
  selector: 'app-inline-card',
  template: `
    <section class="inline-card">
      <h2>INLINE_TITLE</h2>
      <p class="inline-card-body">{{ message }}</p>
    </section>
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .inline-card {
        padding: 0.5rem 1rem;
        border: 1px dashed currentColor;
      }
      .inline-card-body {
        color: var(--inline-card-color, #444);
      }
    `,
  ],
})
export class InlineCard {
  protected readonly message = 'inline-template + inline-styles'
}
