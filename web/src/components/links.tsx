// TanStack Router links rendered as Primer components, with full `to`/`params`/`search`
// typing. See https://tanstack.com/router/latest/docs/framework/react/guide/custom-link
import { ActionList, Breadcrumbs, Button, NavList, UnderlineNav, type ButtonProps } from '@primer/react';
import { createLink, type LinkComponent } from '@tanstack/react-router';
import { forwardRef, type AnchorHTMLAttributes, type ComponentProps } from 'react';

type Anchor = AnchorHTMLAttributes<HTMLAnchorElement>;

const ButtonAnchor = forwardRef<HTMLAnchorElement, Omit<ButtonProps, 'as'> & Anchor>((props, ref) => (
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  <Button as="a" ref={ref as any} {...(props as any)} />
));
const CreatedButtonLink = createLink(ButtonAnchor);
export const ButtonLink: LinkComponent<typeof ButtonAnchor> = (props) => <CreatedButtonLink preload="intent" {...props} />;

const UnderlineItemAnchor = forwardRef<HTMLAnchorElement, Omit<ComponentProps<typeof UnderlineNav.Item>, 'as'> & Anchor>((props, ref) => (
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  <UnderlineNav.Item as="a" ref={ref as any} {...(props as any)} />
));
const CreatedUnderlineLink = createLink(UnderlineItemAnchor);
export const UnderlineNavLink: LinkComponent<typeof UnderlineItemAnchor> = (props) => <CreatedUnderlineLink preload="intent" {...props} />;

const NavListItemAnchor = forwardRef<HTMLAnchorElement, Omit<ComponentProps<typeof NavList.Item>, 'as'> & Anchor>((props, ref) => (
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  <NavList.Item as="a" ref={ref as any} {...(props as any)} />
));
const CreatedNavListLink = createLink(NavListItemAnchor);
export const NavListLink: LinkComponent<typeof NavListItemAnchor> = (props) => <CreatedNavListLink preload="intent" {...props} />;

const ActionListAnchor = forwardRef<HTMLAnchorElement, Omit<ComponentProps<typeof ActionList.LinkItem>, 'as'> & Anchor>((props, ref) => (
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  <ActionList.LinkItem ref={ref as any} {...(props as any)} />
));
const CreatedActionListLink = createLink(ActionListAnchor);
export const ActionListLink: LinkComponent<typeof ActionListAnchor> = (props) => <CreatedActionListLink preload="intent" {...props} />;

const BreadcrumbAnchor = forwardRef<HTMLAnchorElement, Omit<ComponentProps<typeof Breadcrumbs.Item>, 'as'> & Anchor>((props, ref) => (
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  <Breadcrumbs.Item as="a" ref={ref as any} {...(props as any)} />
));
const CreatedBreadcrumbLink = createLink(BreadcrumbAnchor);
export const BreadcrumbLink: LinkComponent<typeof BreadcrumbAnchor> = (props) => <CreatedBreadcrumbLink preload="intent" {...props} />;
