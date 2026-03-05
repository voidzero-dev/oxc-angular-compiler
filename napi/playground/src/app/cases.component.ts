import { Component, Injectable, inject, signal } from '@angular/core'
import { Observable, of, delay } from 'rxjs'

@Injectable({ providedIn: 'root' })
export class CifaPanelService {
  onClose: Observable<boolean> = of(true)
}

@Component({
  selector: 'app-cases',
  template: '<div>Cases: {{ view() }}</div>',
  standalone: true,
})
export class CasesComponent {
  // Field using inject()
  private cifaPanelService = inject(CifaPanelService)

  // Field referencing the inject() field above - CRASHES with native class fields
  // because cifaPanelService may not be initialized yet
  private casesDrawerCloseChangeEvent$ = this.cifaPanelService.onClose.pipe(delay(0))

  // Private field with signal
  #view = signal<string>('home')
  view = this.#view.asReadonly()

  constructor() {
    this.casesDrawerCloseChangeEvent$.subscribe()
    console.log('CasesComponent view:', this.#view())
  }
}
