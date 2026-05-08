/* ═══════════════════════════════════════════════════════════
   JINN LANGUAGE WEBSITE — SCRIPT
   Scroll-driven reveals, nav state
   ═══════════════════════════════════════════════════════════ */

(function () {
    'use strict';

    // ─── NAV SCROLL STATE ────────────────────────────────
    const nav = document.querySelector('.nav');
    let lastY = 0;
    let ticking = false;

    function onScroll() {
        if (!ticking) {
            requestAnimationFrame(() => {
                nav.classList.toggle('nav--scrolled', window.scrollY > 40);
                ticking = false;
            });
            ticking = true;
        }
    }
    window.addEventListener('scroll', onScroll, { passive: true });

    // ─── MOBILE MENU TOGGLE ──────────────────────────────
    const toggle = document.querySelector('.nav__toggle');
    const links = document.querySelector('.nav__links');

    if (toggle && links) {
        toggle.addEventListener('click', () => {
            const expanded = toggle.getAttribute('aria-expanded') === 'true';
            toggle.setAttribute('aria-expanded', String(!expanded));
            links.classList.toggle('open');
            document.body.style.overflow = !expanded ? 'hidden' : '';
        });

        // Close on link click
        links.querySelectorAll('a').forEach(a => {
            a.addEventListener('click', () => {
                toggle.setAttribute('aria-expanded', 'false');
                links.classList.remove('open');
                document.body.style.overflow = '';
            });
        });
    }

    // ─── INTERSECTION OBSERVER: REVEAL ────────────────────
    const reveals = document.querySelectorAll('[data-reveal]');
    if ('IntersectionObserver' in window) {
        const revealObs = new IntersectionObserver(
            (entries) => {
                entries.forEach((entry) => {
                    if (entry.isIntersecting) {
                        entry.target.classList.add('revealed');
                        revealObs.unobserve(entry.target);
                    }
                });
            },
            { threshold: 0.12, rootMargin: '0px 0px -40px 0px' }
        );
        reveals.forEach((el) => revealObs.observe(el));
    } else {
        // Fallback: show all
        reveals.forEach((el) => el.classList.add('revealed'));
    }

    // ─── SMOOTH ANCHOR SCROLLING (backup for Safari) ─────
    document.querySelectorAll('a[href^="#"]').forEach((anchor) => {
        anchor.addEventListener('click', function (e) {
            const id = this.getAttribute('href');
            if (id === '#') return;
            const target = document.querySelector(id);
            if (target) {
                e.preventDefault();
                target.scrollIntoView({ behavior: 'smooth', block: 'start' });
                // Update URL without jump
                history.pushState(null, '', id);
            }
        });
    });

    // ─── ACTIVE NAV LINK HIGHLIGHT ──────────────────────
    const sections = document.querySelectorAll('section[id]');
    const navLinks = document.querySelectorAll('.nav__links a[href^="#"]');

    if ('IntersectionObserver' in window && sections.length) {
        const activeObs = new IntersectionObserver(
            (entries) => {
                entries.forEach((entry) => {
                    if (entry.isIntersecting) {
                        const id = entry.target.getAttribute('id');
                        navLinks.forEach((link) => {
                            link.classList.toggle(
                                'active',
                                link.getAttribute('href') === '#' + id
                            );
                        });
                    }
                });
            },
            { threshold: 0.3, rootMargin: '-80px 0px -40% 0px' }
        );
        sections.forEach((s) => activeObs.observe(s));
    }

    // ─── KEYBOARD: Escape closes mobile menu ────────────
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && links && links.classList.contains('open')) {
            toggle.setAttribute('aria-expanded', 'false');
            links.classList.remove('open');
            document.body.style.overflow = '';
            toggle.focus();
        }
    });

})();
