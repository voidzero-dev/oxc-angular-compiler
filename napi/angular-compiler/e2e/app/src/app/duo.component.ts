import { Component } from '@angular/core'

// Two components in one file. Exercises the per-component HMR pipeline:
// each `@Component` class must get its own template/style cache and HMR
// dispatch slot, addressed by `filePath@ClassName`.

@Component({
  selector: 'app-duo-first',
  template: `
    <section class="duo first">
      <h3>DUO_FIRST_TITLE</h3>
      <p>{{ message }}</p>
    </section>
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .duo.first {
        --duo-first-color: tomato;
        color: var(--duo-first-color);
        padding: 0.5rem;
        border: 1px solid currentColor;
      }
    `,
  ],
})
export class DuoFirst {
  protected readonly message = 'first-component-in-multi-component-file'
}

@Component({
  selector: 'app-duo-second',
  template: `
    <section class="duo second">
      <h3>DUO_SECOND_TITLE</h3>
      <p>{{ message }}</p>
    </section>
  `,
  styles: [
    `
      :host {
        display: block;
      }
      .duo.second {
        --duo-second-color: steelblue;
        color: var(--duo-second-color);
        padding: 0.5rem;
        border: 1px dashed currentColor;
      }
    `,
  ],
})
export class DuoSecond {
  protected readonly message = 'second-component-in-multi-component-file'
}
