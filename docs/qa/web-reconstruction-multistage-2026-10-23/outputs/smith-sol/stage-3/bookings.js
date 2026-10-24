'use strict';
const $ = selector => document.querySelector(selector);
const $$ = selector => [...document.querySelectorAll(selector)];
const bookings = [
  {id:0, initials:'JC', member:'Jamie Chen', space:'Studio 02', date:'2026-10-24T09:00', status:'Confirmed', color:''},
  {id:1, initials:'OR', member:'Olivia Reed', space:'Meeting room A', date:'2026-10-24T11:30', status:'Pending', color:'orange'},
  {id:2, initials:'NW', member:'Noah Williams', space:'Desk 14', date:'2026-10-24T14:00', status:'Confirmed', color:'blue'},
  {id:3, initials:'MP', member:'Maya Patel', space:'Meeting room B', date:'2026-10-25T10:00', status:'Confirmed', color:'purple'},
  {id:4, initials:'LB', member:'Lucas Brown', space:'Studio 01', date:'2026-10-25T13:00', status:'Pending', color:'yellow'},
  {id:5, initials:'EW', member:'Emma Wilson', space:'Desk 08', date:'2026-10-26T09:30', status:'Confirmed', color:'mint'}
];
let statusFilter = 'all';
let toastTimer;
let selectedBooking = null;
let selectedDate = new Date(2026, 9, 23, 12);
let weekStart = new Date(2026, 9, 19, 12);
let calendarMonth = new Date(2026, 9, 1, 12);
let totalBookings = 48;
let pendingApproval = 6;
function notify(message) {
  $('#toast').textContent = message;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { $('#toast').textContent = ''; }, 4000);
}
function element(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}
function icon(name, className = '') {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'icon ' + className);
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
  use.setAttribute('href', '#' + name);
  svg.append(use);
  return svg;
}
function dateKey(date) {
  return `${date.getFullYear()}-${String(date.getMonth()+1).padStart(2,'0')}-${String(date.getDate()).padStart(2,'0')}`;
}
function readableDate(value) {
  const date = new Date(value);
  return date.toLocaleDateString('en-US', {month:'short',day:'numeric'}) + ', ' + date.toLocaleTimeString('en-GB', {hour:'2-digit',minute:'2-digit'});
}
function matchingBookings() {
  const search = $('#booking-search').value.trim().toLowerCase();
  const space = $('#space-filter').value;
  return bookings.filter(booking =>
    (statusFilter === 'all' || booking.status === statusFilter) &&
    (space === 'all' || booking.space === space) &&
    (booking.member.toLowerCase().includes(search) || booking.space.toLowerCase().includes(search))
  );
}
function renderBookings() {
  const matches = matchingBookings();
  const rows = $('#booking-rows');
  rows.replaceChildren();
  for (const booking of matches) {
    const row = element('tr');
    const memberCell = element('td');
    const member = element('div', 'member');
    member.append(element('span', 'avatar ' + booking.color, booking.initials), element('span', '', booking.member));
    memberCell.append(member);
    const statusCell = element('td');
    statusCell.append(element('span', 'status' + (booking.status === 'Pending' ? ' pending' : ''), booking.status));
    const actionCell = element('td');
    const action = element('button', 'row-action');
    action.dataset.booking = booking.id;
    action.setAttribute('aria-label', 'Details for ' + booking.member);
    action.append(icon('more'));
    actionCell.append(action);
    row.append(memberCell, element('td', '', booking.space), element('td', '', readableDate(booking.date)), statusCell, actionCell);
    rows.append(row);
  }
  $('#empty-state').hidden = matches.length !== 0;
  const filtered = statusFilter !== 'all' || $('#booking-search').value.trim() !== '' || $('#space-filter').value !== 'all';
  const total = filtered ? matches.length : totalBookings;
  $('#result-count').textContent = matches.length ? `Showing 1–${matches.length} of ${total} bookings` : 'Showing 0 bookings';
  $('#total-bookings').textContent = totalBookings;
  $('#pending-approval').textContent = pendingApproval;
}
function setStatusFilter(status) {
  statusFilter = status;
  for (const button of $$('[data-status]')) {
    const active = button.dataset.status === status;
    button.classList.toggle('selected', active);
    button.setAttribute('aria-pressed', String(active));
  }
  renderBookings();
}
for (const button of $$('[data-status]')) button.addEventListener('click', () => setStatusFilter(button.dataset.status));
$('#booking-search').addEventListener('input', renderBookings);
$('#space-filter').addEventListener('change', renderBookings);
const bookingDialog = $('#booking-dialog');
$('#new-booking').addEventListener('click', () => bookingDialog.showModal());
$('#cancel-booking').addEventListener('click', () => bookingDialog.close());
$('#booking-form').elements.member.addEventListener('input', event => event.target.setCustomValidity(''));
$('#booking-form').addEventListener('submit', event => {
  event.preventDefault();
  const form = event.currentTarget;
  const data = new FormData(form);
  const name = String(data.get('member')).trim();
  if (!name) {
    form.elements.member.setCustomValidity('Enter a member name.');
    form.elements.member.reportValidity();
    return;
  }
  const booking = {
    id: bookings.length,
    initials: name.split(/\s+/).map(word => word[0]).slice(0,2).join('').toUpperCase(),
    member: name,
    space: String(data.get('space')),
    date: String(data.get('datetime')),
    status: String(data.get('status')),
    color: 'mint'
  };
  bookings.push(booking);
  totalBookings++;
  if (booking.status === 'Pending') pendingApproval++;
  $('#booking-search').value = '';
  $('#space-filter').value = 'all';
  setStatusFilter('all');
  bookingDialog.close();
  form.reset();
  notify('Booking added for ' + name);
});
function showBookingDetails(id) {
  selectedBooking = bookings.find(booking => booking.id === Number(id));
  if (!selectedBooking) return;
  const details = $('#booking-details');
  details.replaceChildren();
  for (const [label, value] of [['Member',selectedBooking.member],['Space',selectedBooking.space],['Date & time',readableDate(selectedBooking.date)],['Status',selectedBooking.status]]) {
    details.append(element('dt','',label), element('dd','',value));
  }
  $('#change-status').textContent = selectedBooking.status === 'Pending' ? 'Confirm booking' : 'Mark pending';
  if (!$('#details-dialog').open) $('#details-dialog').showModal();
}
// Delegate row actions so newly created rows have the same controls.
$('.booking-layout').addEventListener('click', event => {
  const button = event.target.closest('[data-booking]');
  if (button) showBookingDetails(button.dataset.booking);
});
$('#close-details').addEventListener('click', () => $('#details-dialog').close());
$('#change-status').addEventListener('click', () => {
  if (!selectedBooking) return;
  const pending = selectedBooking.status !== 'Pending';
  selectedBooking.status = pending ? 'Pending' : 'Confirmed';
  pendingApproval += pending ? 1 : -1;
  renderBookings();
  showBookingDetails(selectedBooking.id);
  notify('Booking status updated');
});
for (const button of $$('.pagination button')) button.addEventListener('click', () => {
  if (button.dataset.page === '1' || button.dataset.page === 'previous') {
    notify('Showing the first page of available bookings.');
  } else {
    notify('Only the six reference bookings and bookings you add are available locally.');
  }
});
function renderWeek() {
  const days = $('#week-days');
  days.replaceChildren();
  $('#week-month').textContent = weekStart.toLocaleDateString('en-US', {month:'long',year:'numeric'});
  for (let i=0; i<7; i++) {
    const date = new Date(weekStart);
    date.setDate(date.getDate()+i);
    const active = dateKey(date) === dateKey(selectedDate);
    const button = element('button', 'day' + (active ? ' selected' : ''));
    button.dataset.day = dateKey(date);
    button.setAttribute('aria-pressed', String(active));
    button.setAttribute('aria-label', date.toLocaleDateString('en-US', {month:'long',day:'numeric',year:'numeric'}));
    const dot = element('i'); dot.setAttribute('aria-hidden','true');
    button.append(element('span','',date.toLocaleDateString('en-US',{weekday:'short'})),element('strong','',String(date.getDate())),dot);
    days.append(button);
  }
}
function selectDate(date) {
  selectedDate = new Date(date.getFullYear(), date.getMonth(), date.getDate(), 12);
  weekStart = new Date(selectedDate);
  weekStart.setDate(weekStart.getDate() - (weekStart.getDay()+6)%7);
  $('#date-label').textContent = selectedDate.toLocaleDateString('en-US', {month:'short',day:'numeric',year:'numeric'});
  $('#dashboard-date').value = dateKey(selectedDate);
  renderWeek();
}
$('#week-days').addEventListener('click', event => {
  const button = event.target.closest('[data-day]');
  if (button) selectDate(new Date(button.dataset.day+'T12:00:00'));
});
for (const [selector,offset] of [['#week-previous',-7],['#week-next',7]]) $(selector).addEventListener('click', () => {
  weekStart.setDate(weekStart.getDate()+offset);
  renderWeek();
});
function renderCalendar() {
  $('#calendar-month').textContent = calendarMonth.toLocaleDateString('en-US',{month:'long',year:'numeric'});
  const grid = $('#month-days'); grid.replaceChildren();
  const offset = (calendarMonth.getDay()+6)%7;
  for (let i=0;i<offset;i++) grid.append(element('span'));
  const count = new Date(calendarMonth.getFullYear(),calendarMonth.getMonth()+1,0).getDate();
  for (let day=1;day<=count;day++) {
    const date = new Date(calendarMonth.getFullYear(),calendarMonth.getMonth(),day,12);
    const active = dateKey(date) === dateKey(selectedDate);
    const button = element('button',active?'selected':'',String(day));
    button.dataset.day = dateKey(date);
    button.setAttribute('aria-label',date.toLocaleDateString('en-US',{month:'long',day:'numeric',year:'numeric'}));
    button.setAttribute('aria-pressed',String(active));
    grid.append(button);
  }
}
$('#view-calendar').addEventListener('click', () => {
  calendarMonth = new Date(selectedDate.getFullYear(),selectedDate.getMonth(),1,12);
  renderCalendar(); $('#calendar-dialog').showModal();
});
$('#close-calendar').addEventListener('click', () => $('#calendar-dialog').close());
for (const [selector,offset] of [['#calendar-previous',-1],['#calendar-next',1]]) $(selector).addEventListener('click', () => {
  calendarMonth.setMonth(calendarMonth.getMonth()+offset); renderCalendar();
});
$('#month-days').addEventListener('click', event => {
  const button = event.target.closest('[data-day]');
  if (button) {selectDate(new Date(button.dataset.day+'T12:00:00'));$('#calendar-dialog').close();}
});
$('#date-button').addEventListener('click', () => {
  const input = $('#dashboard-date');input.hidden = !input.hidden;if (!input.hidden) input.focus();
});
$('#dashboard-date').addEventListener('change', event => {
  if (event.target.value) {selectDate(new Date(event.target.value+'T12:00:00'));event.target.hidden=true;}
});
document.addEventListener('keydown', event => { if (event.key === 'Escape') $('#dashboard-date').hidden=true; });
for (const button of $$('.nav button')) button.addEventListener('click', () => {
  $$('.nav button, .nav a').forEach(item => {item.classList.remove('active');item.removeAttribute('aria-current');});
  button.classList.add('active');button.setAttribute('aria-current','page');$('#current-page').textContent=button.textContent.trim();
});
$('#notifications').addEventListener('click', () => notify('You’re all caught up. No new notifications.'));
$('#profile').addEventListener('click', () => notify('Signed in as Alex Morgan'));
$('#location').addEventListener('click', () => notify('Alder House is your current workspace'));
