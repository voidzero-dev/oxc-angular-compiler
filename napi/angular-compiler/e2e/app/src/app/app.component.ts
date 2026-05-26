import { Component, signal } from '@angular/core'

import { Card } from './card.component'
import { DuoFirst, DuoSecond } from './duo.component'
import { InlineCard } from './inline-card.component'
import { UTIL_VALUE } from './util'

@Component({
  selector: 'app-root',
  templateUrl: './app.html',
  styleUrl: './app.css',
  imports: [Card, InlineCard, DuoFirst, DuoSecond],
})
export class App {
  protected readonly title = signal('E2E_TITLE')
  protected readonly utilValue = UTIL_VALUE
}
