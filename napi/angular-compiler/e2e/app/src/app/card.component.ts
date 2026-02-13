import { Component, input } from '@angular/core'

@Component({
  selector: 'app-card',
  templateUrl: './card.html',
  styleUrl: './card.css',
})
export class Card {
  cardTitle = input.required<string>()
  cardValue = input(0)
}
