import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ErrorDetails from '@/ui/ErrorDetails.vue'
import { describeError } from '@/api/wot/errors'

// Response bodies in the shapes a teta-wot v0.1.0 server sends in its default
// (`tetathing`) profile: crates/teta-wot-http/src/render.rs and problem.rs.
const notFound = { detail: 'No action found with the name "focus".' }
const invalid = {
  detail: [
    { type: 'int_parsing', loc: ['body', 'x'], msg: 'Input should be a valid integer', input: 'a' },
    { type: 'missing', loc: ['body', 'rows', 2, 'name'], msg: 'Field required', input: {} },
    { type: 'missing', loc: ['body'], msg: 'Field required', input: null },
    { type: 'int_parsing', loc: ['query', 'limit'], msg: 'Bad limit', input: 'x' },
  ],
}
const problem = {
  detail: 'The global lock is held by another action.',
  type: 'https://www.tetaprecision.com/api/exceptions/index.html#teta_wot.exceptions.GlobalLockBusyError',
  status: 503,
  title: 'GlobalLockBusyError',
  instance: null,
}

describe('describeError', () => {
  it('reads a {detail} message', () => {
    expect(describeError(notFound)).toEqual({ message: notFound.detail, issues: [] })
  })

  it('reads a validation list, naming where each issue is', () => {
    expect(describeError(invalid)).toEqual({
      title: '4 invalid inputs',
      issues: [
        { field: 'x', message: 'Input should be a valid integer' },
        { field: 'rows[2].name', message: 'Field required' },
        { field: '', message: 'Field required' },
        { field: 'limit', message: 'Bad limit' },
      ],
    })
    expect(describeError({ detail: [invalid.detail[0]] }).title).toBe('1 invalid input')
  })

  it('reads a problem', () => {
    expect(describeError(problem)).toEqual({
      title: 'GlobalLockBusyError',
      message: problem.detail,
      issues: [],
    })
  })

  it('reads text and errors', () => {
    expect(describeError('Offline')).toEqual({ message: 'Offline', issues: [] })
    expect(describeError(new TypeError('Failed to fetch'))).toEqual({
      message: 'Failed to fetch',
      issues: [],
    })
  })

  it('shows anything else as JSON', () => {
    expect(describeError({ unexpected: true }).json).toBe('{\n  "unexpected": true\n}')
    expect(describeError([1, 2]).json).toBe('[\n  1,\n  2\n]')
    expect(describeError(undefined).json).toBe('undefined')
  })
})

describe('ErrorDetails', () => {
  it('shows a validation list as a list of fields and messages', () => {
    const wrapper = mount(ErrorDetails, { props: { error: invalid } })
    expect(wrapper.get('.error-details__title').text()).toBe('4 invalid inputs')
    const items = wrapper.findAll('li')
    expect(items.map((item) => item.find('code').exists() && item.get('code').text())).toEqual([
      'x',
      'rows[2].name',
      false,
      'limit',
    ])
    expect(items[0]!.text()).toBe('x Input should be a valid integer')
  })

  it("shows a problem's name and message", () => {
    const wrapper = mount(ErrorDetails, { props: { error: problem } })
    expect(wrapper.findAll('p').map((p) => p.text())).toEqual([
      'GlobalLockBusyError',
      problem.detail,
    ])
    expect(wrapper.find('ul').exists()).toBe(false)
  })

  it('shows an unknown shape as JSON', () => {
    expect(
      mount(ErrorDetails, { props: { error: { code: 7 } } })
        .get('pre')
        .text(),
    ).toBe('{\n  "code": 7\n}')
  })
})
