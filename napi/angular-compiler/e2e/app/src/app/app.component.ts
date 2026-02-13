import { Component, signal } from '@angular/core'

import { Card } from './card.component'

@Component({
  selector: 'app-root',
  templateUrl: './app.html',
  styleUrl: './app.css',
  imports: [Card],
})
export class App {
  protected readonly title = signal('E2E_TITLE')
}
