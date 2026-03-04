import { Component, signal } from '@angular/core'
import { RouterOutlet } from '@angular/router'

import { Log } from './custom-decorator'

@Log('App component loaded')
@Component({
  selector: 'app-root',
  imports: [RouterOutlet],
  templateUrl: './app.html',
  styleUrl: './app.css',
})
export class App {
  protected readonly title = signal('Angular')
}
