import { nextTick } from 'vue'
import { expect, it, vi } from 'vitest'
import { createStreamPool } from '../src/streamPool'

it('limits leases and promotes visible waiters after the current player releases', async () => {
  const reserve = createStreamPool()
  const states = Array.from({ length: 9 }, () => vi.fn())
  const release = states.map(reserve)
  await nextTick()
  expect(states.slice(0, 5).every(notify => notify.mock.calls[0]?.[0] === true)).toBe(true)
  expect(states.slice(5).every(notify => notify.mock.calls.length === 0)).toBe(true)

  release[5]()
  release[0]()
  release[0]()
  expect(states[0].mock.calls).toEqual([[true], [false]])
  expect(states[6]).not.toHaveBeenCalled()
  await nextTick()
  expect(states[5]).not.toHaveBeenCalled()
  expect(states[6]).toHaveBeenCalledExactlyOnceWith(true)
  expect(states[7]).not.toHaveBeenCalled()
})
